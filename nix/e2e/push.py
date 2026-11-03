import re
import shlex
import time

targets = {"boron": boron, "cerium": cerium, "radon": radon}
by_name = {s["name"]: s for s in scenarios}
ansi = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]|\x1b[()][A-Z0-9]|\r")


def plain(text: str) -> str:
    return ansi.sub("", text)


class Push:
    def __init__(self, name: str, env: str = ""):
        self.name = name
        flake = f"/tmp/scenarios/{name}"
        client.succeed(
            f"rm -rf {flake} && cp -rL /etc/scenarios/{name} {flake} && chmod -R u+w {flake}"
        )
        command = f"snow push {by_name[name]['args']}".strip()
        # snow names its log by the second; one log at a time can't be mistaken
        client.succeed("rm -f /tmp/snow-*.log")
        started = time.monotonic()
        self.status, output = client.execute(
            f"cd {flake} && {env} record /tmp/casts/{name}.cast 100 24 {command}"
        )
        self.seconds = time.monotonic() - started
        self.output = plain(output)
        client.succeed(f"cp /tmp/snow-*.log /tmp/casts/{name}.log")
        self.log = client.succeed(f"cat /tmp/casts/{name}.log")

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
client.succeed("mkdir -p /tmp/casts /tmp/scenarios")
key = client.succeed("cat /root/.ssh/id_ed25519.pub").strip()
for name, machine in targets.items():
    machine.wait_for_unit("sshd.service")
    machine.succeed(f"install -d -m 700 /root/.ssh && echo {shlex.quote(key)} > /root/.ssh/authorized_keys")
    client.wait_until_succeeds(f"ssh -o BatchMode=yes root@{name} true", timeout=60)
base = {name: running(name) for name in targets}

# Each push expects the hosts as the one before left them
try:
    with subtest("three hosts in two phases switch to a new system"):
        push = Push("switch")
        push.succeeded()
        push.has("Pushing 3 hosts in 2 phases:", "p1  boron", "p2  cerium, radon", "✓ Phase p1", "✓ Phase p2")
        push.logged("boron: activate started", "boron: deployed", "cerium: deployed", "radon: deployed")
        assert push.log.index("boron: deployed") < push.log.index("cerium: build started"), push.log
        for host in targets:
            runs(host, "v1")

    with subtest("a host that must reboot boots into the new system"):
        before = boot_id("boron")
        push = Push("reboot")
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
        push = Push("activation-failure")
        push.failed()
        push.has("cerium", "1/3 failed")
        push.logged("cerium: failed at activate:", "boron: deployed", "radon: deployed")
        # NixOS keeps a system whose services failed to start
        runs("cerium", "broken")
        runs("boron", "v3")
        runs("radon", "v2")

    with subtest("a failing pre-deploy hook keeps its host from activating"):
        push = Push("hook-failure")
        push.failed()
        push.has("radon", "wait-storage-healthy")
        push.logged("radon: failed at", "storage on radon is degraded", "cerium: deployed")
        runs("cerium", "v1")
        runs("radon", "v2")

    with subtest("a local hook whose program is missing stops the push before building"):
        push = Push("missing-hook")
        push.failed()
        # The error wraps the path after /nix/store/
        push.has(by_name["missing-hook"]["missingPath"].removeprefix("/nix/store/"))
        assert "build started" not in push.log, push.log
        runs("boron", "v3")
        runs("cerium", "v1")

    with subtest("a host off the network fails on its own after ssh times out"):
        cerium.block()
        try:
            push = Push("unreachable")
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
            push = Push("hung-agent", env="SSH_AUTH_SOCK=/tmp/hung-agent.sock")
        finally:
            client.succeed("systemctl stop hung-agent")
        push.failed()
        push.has("ssh-agent did not answer within 10s")
        assert push.seconds < 60, push.seconds
        runs("cerium", "v1")

    with subtest("two hosts in a serial group take turns"):
        push = Push("serial-group")
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
        push = Push("reboot-deadline")
        push.failed()
        push.has("radon")
        push.logged("radon: failed at reboot:", "did not come back online within 20 seconds")
        radon.wait_for_shutdown()
finally:
    # Kept even when a scenario fails, to see what it showed
    client.copy_from_vm("/tmp/casts", "")
