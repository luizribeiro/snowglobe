//! Per-phase host scheduling: the global concurrency limit and serial groups
//!
//! Build, copy and planning run under a concurrency permit. A host's disruptive
//! section (hooks, activation, reboot) first takes its serial group's slot and
//! only then a permit, so hosts queued behind a group never hold a permit that
//! ungrouped hosts could use.

use crate::host::HostMetadata;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::{OwnedMutexGuard, OwnedSemaphorePermit, Semaphore};

pub(super) struct PhaseScheduler {
    permits: Arc<Semaphore>,
    groups: HashMap<String, Arc<SerialGroup>>,
}

#[derive(Default)]
struct SerialGroup {
    slot: Arc<tokio::sync::Mutex<()>>,
    state: Mutex<GroupState>,
}

#[derive(Default)]
struct GroupState {
    holder: Option<String>,
    halted_by: Option<String>,
}

/// The host's serial group stopped after a member failed
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GroupHalted {
    pub group: String,
    pub failed_host: String,
}

/// A host's turn to run its disruptive section
///
/// Dropping it without calling [`Section::succeeded`] halts the host's group,
/// so a failure is recorded before the next member can take the slot.
pub(super) struct Section {
    hostname: String,
    halts_group_on_failure: bool,
    group: Option<(Arc<SerialGroup>, OwnedMutexGuard<()>)>,
    succeeded: bool,
    _permit: OwnedSemaphorePermit,
}

impl SerialGroup {
    fn state(&self) -> std::sync::MutexGuard<'_, GroupState> {
        self.state.lock().expect("serial group state poisoned")
    }

    fn halt(&self, hostname: &str) {
        self.state()
            .halted_by
            .get_or_insert_with(|| hostname.to_string());
    }

    fn halted(&self, group: &str) -> Option<GroupHalted> {
        self.state()
            .halted_by
            .clone()
            .map(|failed_host| GroupHalted {
                group: group.to_string(),
                failed_host,
            })
    }
}

impl PhaseScheduler {
    pub fn new(concurrency: usize, hosts: &[HostMetadata]) -> Self {
        let groups = hosts
            .iter()
            .filter_map(|host| host.serial_group.clone())
            .map(|group| (group, Arc::default()))
            .collect();
        Self {
            permits: Arc::new(Semaphore::new(concurrency)),
            groups,
        }
    }

    pub async fn permit(&self) -> OwnedSemaphorePermit {
        Arc::clone(&self.permits)
            .acquire_owned()
            .await
            .expect("concurrency semaphore closed")
    }

    fn group_of<'a>(&'a self, host: &'a HostMetadata) -> Option<(&'a str, &'a Arc<SerialGroup>)> {
        let name = host.serial_group.as_deref()?;
        self.groups.get(name).map(|group| (name, group))
    }

    /// Wait for the host's turn in its serial group (if any), then a permit
    ///
    /// `on_wait` is called with the group and current holder when the host
    /// has to queue.
    pub async fn enter_section(
        &self,
        host: &HostMetadata,
        on_wait: impl FnOnce(&str, Option<&str>),
    ) -> Result<Section, GroupHalted> {
        let Some((name, group)) = self.group_of(host) else {
            return Ok(Section {
                hostname: host.name.clone(),
                halts_group_on_failure: halts_group_on_failure(host),
                group: None,
                succeeded: false,
                _permit: self.permit().await,
            });
        };

        if let Some(halted) = group.halted(name) {
            return Err(halted);
        }
        let guard = match Arc::clone(&group.slot).try_lock_owned() {
            Ok(guard) => guard,
            Err(_) => {
                let holder = group.state().holder.clone();
                on_wait(name, holder.as_deref());
                Arc::clone(&group.slot).lock_owned().await
            }
        };
        if let Some(halted) = group.halted(name) {
            return Err(halted);
        }
        group.state().holder = Some(host.name.clone());
        let section = Section {
            hostname: host.name.clone(),
            halts_group_on_failure: halts_group_on_failure(host),
            group: Some((Arc::clone(group), guard)),
            succeeded: false,
            _permit: self.permit().await,
        };

        // A member may also fail while this host waits for a permit
        match group.halted(name) {
            Some(halted) => Err(halted),
            None => Ok(section),
        }
    }

    /// Stop the host's serial group from starting any more members
    pub fn halt_group_of(&self, host: &HostMetadata) {
        if !halts_group_on_failure(host) {
            return;
        }
        if let Some((_, group)) = self.group_of(host) {
            group.halt(&host.name);
        }
    }
}

/// An optional host's failure mustn't fail its phase, which skipping the rest
/// of its group would
fn halts_group_on_failure(host: &HostMetadata) -> bool {
    !host.optional
}

impl Section {
    pub fn succeeded(mut self) {
        self.succeeded = true;
    }
}

impl Drop for Section {
    fn drop(&mut self) {
        if let Some((group, _guard)) = &self.group {
            if !self.succeeded && self.halts_group_on_failure {
                group.halt(&self.hostname);
            }
            group.state().holder = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nix::SystemType;
    use crate::reboot::RebootPolicy;
    use std::time::Duration;

    fn host(name: &str, group: Option<&str>) -> HostMetadata {
        HostMetadata {
            name: name.to_string(),
            phase: Some("p2".to_string()),
            tags: vec![],
            fqdn: format!("{}.example.com", name),
            reboot_policy: RebootPolicy::Always,
            system_type: SystemType::NixOS,
            system: None,
            hooks: vec![],
            serial_group: group.map(str::to_string),
            ssh_port: None,
            optional: false,
            ignored_failed_units: Vec::new(),
        }
    }

    fn no_wait(_: &str, _: Option<&str>) {}

    async fn is_blocked<T>(future: impl std::future::Future<Output = T>) -> bool {
        tokio::time::timeout(Duration::from_millis(50), future)
            .await
            .is_err()
    }

    async fn without_blocking<T>(future: impl std::future::Future<Output = T>) -> T {
        tokio::time::timeout(Duration::from_millis(50), future)
            .await
            .expect("blocked")
    }

    #[tokio::test]
    async fn test_same_group_waits_for_the_holder() {
        let praseodymium = host("praseodymium", Some("kubernetes"));
        let promethium = host("promethium", Some("kubernetes"));
        let scheduler = PhaseScheduler::new(3, &[praseodymium.clone(), promethium.clone()]);

        let section = scheduler
            .enter_section(&praseodymium, no_wait)
            .await
            .unwrap();
        let mut waited_on = None;
        assert!(
            is_blocked(scheduler.enter_section(&promethium, |group, holder| {
                waited_on = Some((group.to_string(), holder.map(str::to_string)));
            }))
            .await
        );
        assert_eq!(
            waited_on,
            Some(("kubernetes".to_string(), Some("praseodymium".to_string())))
        );

        section.succeeded();
        assert!(scheduler.enter_section(&promethium, no_wait).await.is_ok());
    }

    #[tokio::test]
    async fn test_different_groups_and_ungrouped_hosts_do_not_wait() {
        let praseodymium = host("praseodymium", Some("kubernetes"));
        let consul = host("argon", Some("consul"));
        let hydrogen = host("hydrogen", None);
        let scheduler =
            PhaseScheduler::new(3, &[praseodymium.clone(), consul.clone(), hydrogen.clone()]);

        let _held = scheduler
            .enter_section(&praseodymium, no_wait)
            .await
            .unwrap();

        let _consul = without_blocking(scheduler.enter_section(&consul, no_wait)).await;
        let _hydrogen = without_blocking(scheduler.enter_section(&hydrogen, no_wait)).await;
    }

    #[tokio::test]
    async fn test_failed_section_halts_waiting_members() {
        let praseodymium = host("praseodymium", Some("kubernetes"));
        let promethium = host("promethium", Some("kubernetes"));
        let scheduler = PhaseScheduler::new(3, &[praseodymium.clone(), promethium.clone()]);

        let section = scheduler
            .enter_section(&praseodymium, no_wait)
            .await
            .unwrap();
        let waiting = scheduler.enter_section(&promethium, no_wait);
        tokio::pin!(waiting);
        assert!(is_blocked(&mut waiting).await);
        drop(section);

        assert_eq!(
            waiting.await.err(),
            Some(GroupHalted {
                group: "kubernetes".to_string(),
                failed_host: "praseodymium".to_string(),
            })
        );
    }

    #[tokio::test]
    async fn test_failed_optional_host_lets_the_next_member_in() {
        let praseodymium = HostMetadata {
            optional: true,
            ..host("praseodymium", Some("kubernetes"))
        };
        let promethium = host("promethium", Some("kubernetes"));
        let scheduler = PhaseScheduler::new(3, &[praseodymium.clone(), promethium.clone()]);

        let section = scheduler
            .enter_section(&praseodymium, no_wait)
            .await
            .unwrap();
        let waiting = scheduler.enter_section(&promethium, no_wait);
        tokio::pin!(waiting);
        assert!(is_blocked(&mut waiting).await);
        drop(section);
        scheduler.halt_group_of(&praseodymium);

        assert!(without_blocking(waiting).await.is_ok());
    }

    #[tokio::test]
    async fn test_halted_group_refuses_members_immediately() {
        let praseodymium = host("praseodymium", Some("kubernetes"));
        let promethium = host("promethium", Some("kubernetes"));
        let scheduler = PhaseScheduler::new(3, &[praseodymium.clone(), promethium.clone()]);

        scheduler.halt_group_of(&praseodymium);

        assert!(scheduler.enter_section(&promethium, no_wait).await.is_err());
    }

    #[tokio::test]
    async fn test_waiting_for_a_group_holds_no_permit() {
        let praseodymium = host("praseodymium", Some("kubernetes"));
        let promethium = host("promethium", Some("kubernetes"));
        let hydrogen = host("hydrogen", None);
        let scheduler = PhaseScheduler::new(
            2,
            &[praseodymium.clone(), promethium.clone(), hydrogen.clone()],
        );

        let _held = scheduler
            .enter_section(&praseodymium, no_wait)
            .await
            .unwrap();
        let queued = scheduler.enter_section(&promethium, no_wait);
        tokio::pin!(queued);
        assert!(is_blocked(&mut queued).await);

        let _hydrogen = without_blocking(scheduler.enter_section(&hydrogen, no_wait)).await;
    }

    #[tokio::test]
    async fn test_halt_while_waiting_for_a_permit_is_honored() {
        let praseodymium = host("praseodymium", Some("kubernetes"));
        let promethium = host("promethium", Some("kubernetes"));
        let scheduler = PhaseScheduler::new(1, &[praseodymium.clone(), promethium.clone()]);

        let building = scheduler.permit().await;
        let entering = scheduler.enter_section(&promethium, no_wait);
        tokio::pin!(entering);
        assert!(is_blocked(&mut entering).await);
        scheduler.halt_group_of(&praseodymium);
        drop(building);

        assert!(entering.await.is_err());
    }
}
