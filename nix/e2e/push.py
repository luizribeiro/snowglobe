import re
import shlex
import time
from typing import Any, cast

targets = {"boron": boron, "cerium": cerium, "radon": radon}
nodes = {"client": client, **targets}
# The machine the client stands in for in the recorded prompt
DEPLOYER = "sodium"
by_name = {s["name"]: s for s in cast(list[dict[str, Any]], scenarios)}
ansi = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]|\x1b[()][A-Z0-9]|\r")


def plain(text: str) -> str:
    return ansi.sub("", text)


class Recording:
    """Runs a scenario's snow command where it runs, recorded under the prompt
    the maintainer would see there"""

    def __init__(self, name: str):
        self.name = name
        self.scenario = by_name[name]
        self.on = self.scenario.get("on", "client")
        self.machine = nodes[self.on]
        self.flake = f"/tmp/scenarios/{name}"
        self.stage_flake()
        # snow names its log by the second; one log at a time can't be mistaken
        self.machine.succeed("rm -f /tmp/snow-*.log")

    def stage_flake(self) -> None:
        source = client.succeed(f"readlink -f /etc/scenarios/{self.name}").strip()
        if self.on != "client":
            client.succeed(f"nix copy --no-check-sigs --to ssh://root@{self.on} {source}")
        self.machine.succeed(
            f"rm -rf {self.flake} && cp -rL {source} {self.flake} && chmod -R u+w {self.flake}"
        )

    def record_command(self) -> str:
        host = DEPLOYER if self.on == "client" else self.on
        answers = "".join(
            f" --answer {shlex.quote(text)} {shlex.quote(keys)}"
            for text, keys in self.scenario.get("answers", [])
        )
        command = f"snow {self.scenario.get('command', 'push')} {self.scenario['args']}"
        return (
            f"record --prompt luiz@{host}:ops{answers} "
            f"/var/lib/casts/{self.name}.cast 100 24 -- {command.strip()}"
        )

    def run(self, env: str = "") -> "Recording":
        started = time.monotonic()
        self.status, output = self.machine.execute(
            f"cd {self.flake} && {env} {self.record_command()}"
        )
        self.seconds = time.monotonic() - started
        self.output = plain(output)
        self.collect()
        return self

    def start(self) -> None:
        """Runs in the background, for a command that takes its machine down"""
        self.machine.succeed(
            f"systemd-run --unit=record-{self.name} --working-directory={self.flake} "
            "-E PATH=/run/wrappers/bin:/run/current-system/sw/bin -E HOME=/root "
            f"/run/current-system/sw/bin/{self.record_command()}"
        )

    def collect(self) -> None:
        self.machine.succeed(f"cp /tmp/snow-*.log /var/lib/casts/{self.name}.log")
        self.log = self.machine.succeed(f"cat /var/lib/casts/{self.name}.log")

    def has(self, *texts: str) -> None:
        for text in texts:
            assert text in self.output, (text, self.output)

    def logged(self, *texts: str) -> None:
        for text in texts:
            assert text in self.log, (text, self.log)

    def failed(self) -> None:
        assert self.status != 0, self.output

    def succeeded(self) -> None:
        assert self.status == 0, self.output


def running(host: str) -> str:
    return targets[host].succeed("readlink -f /run/current-system").strip()


def boots_into(host: str) -> str:
    return targets[host].succeed("readlink -f /nix/var/nix/profiles/system").strip()


def boot_id(host: str) -> str:
    return targets[host].succeed("cat /proc/sys/kernel/random/boot_id").strip()


def runs(host: str, variant: str) -> None:
    assert running(host) == variants[host][variant], (host, variant, running(host))


# A guest that reboots would otherwise power off, and boron has to come back
boron.start(allow_reboot=True)
start_all()
client.succeed("ssh-keygen -q -t ed25519 -N '' -f /root/.ssh/id_ed25519")
for machine in (client, boron):
    machine.succeed("mkdir -p /var/lib/casts /tmp/scenarios")
key = client.succeed("cat /root/.ssh/id_ed25519.pub").strip()
for name, machine in targets.items():
    machine.wait_for_unit("sshd.service")
    machine.succeed(f"install -d -m 700 /root/.ssh && echo {shlex.quote(key)} > /root/.ssh/authorized_keys")
    client.wait_until_succeeds(f"ssh -o BatchMode=yes root@{name} true", timeout=60)
base = {name: running(name) for name in targets}

# Each push expects the hosts as the one before left them
try:
    with subtest("three hosts in two phases switch to a new system"):
        push = Recording("switch").run()
        push.succeeded()
        push.has("Pushing 3 hosts in 2 phases:", "p1  boron", "p2  cerium, radon", "✓ Phase p1", "✓ Phase p2")
        push.logged("boron: activate started", "boron: deployed", "cerium: deployed", "radon: deployed")
        assert push.log.index("boron: deployed") < push.log.index("cerium: build started"), push.log
        for host in targets:
            runs(host, "v1")

    with subtest("a host that must reboot boots into the new system"):
        before = boot_id("boron")
        push = Recording("reboot").run()
        push.succeeded()
        push.logged("boron: reboot started", "boron: reboot finished in", "boron: deployed")
        # A test VM boots the kernel it started with, so it runs its first system
        # again; the profile shows what it was told to boot
        # The driver doesn't know the guest rebooted itself
        boron.connected = False
        boron.wait_for_unit("multi-user.target")
        assert boot_id("boron") != before
        assert boots_into("boron") == variants["boron"]["v2"], boots_into("boron")

    with subtest("one host fails to activate while the others succeed"):
        push = Recording("activation-failure").run()
        push.failed()
        push.has("cerium", "1/3 failed")
        push.logged("cerium: failed at activate:", "boron: deployed", "radon: deployed")
        # NixOS keeps a system whose services failed to start
        runs("cerium", "broken")
        runs("boron", "v3")
        runs("radon", "v2")

    with subtest("a failing pre-deploy hook keeps its host from activating"):
        push = Recording("hook-failure").run()
        push.failed()
        push.has("radon", "wait-storage-healthy")
        push.logged("radon: failed at", "storage on radon is degraded", "cerium: deployed")
        runs("cerium", "v1")
        runs("radon", "v2")

    with subtest("a local hook whose program is missing stops the push before building"):
        push = Recording("missing-hook").run()
        push.failed()
        # The error wraps the path after /nix/store/
        push.has(by_name["missing-hook"]["missingPath"].removeprefix("/nix/store/"))
        assert "build started" not in push.log, push.log
        runs("boron", "v3")
        runs("cerium", "v1")

    with subtest("a host off the network fails on its own after ssh times out"):
        cerium.block()
        try:
            push = Recording("unreachable").run()
        finally:
            cerium.unblock()
        push.failed()
        push.has("cerium")
        push.logged("cerium: failed at", "boron: deployed")
        runs("boron", "v1")
        runs("cerium", "v1")

    with subtest("a hung ssh-agent stops the push at once"):
        client.succeed(
            "systemd-run --unit=hung-agent socat UNIX-LISTEN:/tmp/hung-agent.sock,fork SYSTEM:'exec /run/current-system/sw/bin/sleep 600'"
        )
        client.wait_for_file("/tmp/hung-agent.sock")
        try:
            push = Recording("hung-agent").run(env="SSH_AUTH_SOCK=/tmp/hung-agent.sock")
        finally:
            client.succeed("systemctl stop hung-agent")
        push.failed()
        push.has("ssh-agent did not answer within 10s")
        assert push.seconds < 60, push.seconds
        runs("cerium", "v1")

    with subtest("two hosts in a serial group take turns"):
        push = Recording("serial-group").run()
        push.succeeded()
        waiting = re.search(r"(\w+): waiting for serial group 'storage' \(held by (\w+)\)", push.log)
        assert waiting, push.log
        waiter, holder = waiting.groups()
        assert {waiter, holder} == {"cerium", "radon"}, push.log
        assert push.log.index(f"{holder}: deployed") < push.log.index(f"{waiter}: activate started"), push.log
        runs("cerium", "v2")
        runs("radon", "v3")

    with subtest("a rebooted host that never comes back fails at the deadline"):
        # radon powers off instead of rebooting, as test VMs do by default
        push = Recording("reboot-deadline").run()
        push.failed()
        push.has("radon")
        push.logged("radon: failed at reboot:", "did not come back online within 20s")
        radon.wait_for_shutdown()

    with subtest("snow rebuild asks before switching without a reboot"):
        before = boots_into("boron")
        rebuild = Recording("rebuild-cancel").run()
        rebuild.succeeded()
        rebuild.has("Reboot needed:", "• initrd changed", "What now?", "mixes old and new parts", "Switch anyway?", "⊘ Cancelled, nothing changed")
        rebuild.logged("boron: reboot required: initrd changed", "boron: chose to cancel")
        assert boots_into("boron") == before, boots_into("boron")

    with subtest("snow rebuild sets the new system for the next boot"):
        running_before = running("boron")
        rebuild = Recording("rebuild-boot").run()
        rebuild.succeeded()
        rebuild.has("The new system starts at the next boot")
        rebuild.logged("boron: built", "boron: chose to activate on the next boot", "boron: activated (Boot mode)")
        assert boots_into("boron") == variants["boron"]["initrd"], boots_into("boron")
        assert running("boron") == running_before, running("boron")

    with subtest("snow rebuild reboots into the new system"):
        before = boot_id("boron")
        rebuild = Recording("rebuild-reboot")
        rebuild.start()
        # Asked from the client: boron's own shell goes down with it
        client.wait_until_succeeds(
            f"ssh -o ConnectTimeout=5 root@boron 'test $(cat /proc/sys/kernel/random/boot_id) != {before}'",
            timeout=300,
        )
        # The driver's shell on boron died with the reboot
        boron.connected = False
        boron.wait_for_unit("multi-user.target")
        rebuild.collect()
        rebuild.logged("boron: chose to reboot now", "boron: activated (Boot mode)", "boron: rebooting")
        assert boots_into("boron") == variants["boron"]["initrd"], boots_into("boron")
        recorded = boron.succeed("cat /var/lib/casts/rebuild-reboot.cast")
        assert "Rebooting" in recorded, recorded
finally:
    # Kept even when a scenario fails, to see what it showed
    client.execute("scp -q -o ConnectTimeout=10 'root@boron:/var/lib/casts/*' /var/lib/casts/")
    client.copy_from_vm("/var/lib/casts", "")
