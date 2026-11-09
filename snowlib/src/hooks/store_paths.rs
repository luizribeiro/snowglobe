//! Finding Nix store paths referenced by hook commands

const STORE_DIR: &str = "/nix/store/";
const HASH_LEN: usize = 32;

fn is_hash_char(c: char) -> bool {
    // Nix's base32 alphabet: digits and lowercase letters except e, o, t, u
    c.is_ascii_digit() || (c.is_ascii_lowercase() && !matches!(c, 'e' | 'o' | 't' | 'u'))
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.' | '_' | '?' | '=')
}

/// Store paths (`/nix/store/<hash>-<name>`) mentioned anywhere in `command`
pub(crate) fn find_store_paths(command: &str) -> Vec<&str> {
    let mut paths = Vec::new();
    let mut rest = command;
    while let Some(start) = rest.find(STORE_DIR) {
        let candidate = &rest[start..];
        let after_dir = &candidate[STORE_DIR.len()..];
        let hash_ok = after_dir
            .get(..HASH_LEN)
            .is_some_and(|hash| hash.chars().all(is_hash_char))
            && after_dir[HASH_LEN..].starts_with('-');
        if hash_ok {
            let name_len = after_dir[HASH_LEN + 1..]
                .find(|c: char| !is_name_char(c))
                .unwrap_or(after_dir.len() - HASH_LEN - 1);
            if name_len > 0 {
                paths.push(&candidate[..STORE_DIR.len() + HASH_LEN + 1 + name_len]);
            }
        }
        rest = &candidate[STORE_DIR.len()..];
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    const KUBECTL: &str = "/nix/store/vcy97mavll1fwawj02qr7lkkl0kl2q6y-kubectl-1.34.2";
    const GATE: &str = "/nix/store/ywilmysgc3p103v1y4hhzbhb3v0r0mkh-kubernetes-storage-healthy";

    #[test]
    fn test_finds_leading_store_path() {
        let command = format!("{}/bin/kubectl drain pomegranates", KUBECTL);

        assert_eq!(find_store_paths(&command), vec![KUBECTL]);
    }

    #[test]
    fn test_finds_store_paths_anywhere_in_a_shell_command() {
        let command = format!(
            "PATH={}/bin:$PATH '{}/bin/kubernetes-storage-healthy' --wait && echo ok",
            KUBECTL, GATE
        );

        assert_eq!(find_store_paths(&command), vec![KUBECTL, GATE]);
    }

    #[test]
    fn test_ignores_commands_without_store_paths() {
        assert!(find_store_paths("kubectl uncordon pomegranates").is_empty());
        assert!(find_store_paths("ls /nix/store/").is_empty());
    }

    #[test]
    fn test_finds_store_path_at_end_of_command() {
        let command = format!("exec {}", KUBECTL);

        assert_eq!(find_store_paths(&command), vec![KUBECTL]);
    }

    #[test]
    fn test_finds_adjacent_store_paths() {
        let command = format!("{}{}", KUBECTL, GATE);

        assert_eq!(find_store_paths(&command), vec![KUBECTL, GATE]);
    }

    #[test]
    fn test_ignores_store_path_without_name() {
        assert!(find_store_paths("/nix/store/vcy97mavll1fwawj02qr7lkkl0kl2q6y-").is_empty());
        assert!(find_store_paths("/nix/store/vcy97mavll1fwawj02qr7lkkl0kl2q6y-/bin").is_empty());
        assert!(
            find_store_paths("/nix/store/vcy97mavll1fwawj02qr7lkkl0kl2q6yk-kubectl").is_empty()
        );
    }

    #[test]
    fn test_handles_non_ascii_text_after_store_dir() {
        let command = format!("echo /nix/store/{}é done", "a".repeat(31));

        assert!(find_store_paths(&command).is_empty());
    }

    #[test]
    fn test_ignores_malformed_hashes() {
        assert!(find_store_paths("/nix/store/not-a-hash-kubectl/bin/kubectl").is_empty());
        assert!(find_store_paths("/nix/store/eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee-kubectl").is_empty());
    }
}
