---
title: "Connect to your hosts"
group: "Guides"
weight: 35
description: "How snow finds each host's address, which SSH user and port it uses, and how to shape that with your SSH config."
---

This guide explains how `snow` reaches each host over SSH, and how to control the address, user, port, key and route with your ordinary OpenSSH configuration. When it works, `ssh -o BatchMode=yes <address> true` succeeds for every host, and so does `snow deploy`.

## How snow connects

snow does not contain its own SSH client. Every connection goes through the `ssh` program on your `PATH`, and the copy goes through `nix copy`, which calls the same `ssh`. Your `~/.ssh/config`, `/etc/ssh/ssh_config`, `known_hosts`, keys and agent all apply exactly as they do when you type `ssh` yourself.

There is nothing to install on the hosts beyond the snowglobe module and an SSH server, and there is no snow-specific connection configuration to keep in sync. If plain `ssh` reaches a host, snow can too.

## The address snow connects to

For each host, snow evaluates the configuration and connects to:

- `networking.hostName`, when `networking.domain` is unset or empty;
- `networking.hostName` + `.` + `networking.domain`, when a domain is set.

The name of the flake output does not matter. With the configuration below, snow connects to `web-01.example.com` even though you select the host as `web1`:

```nix
{
  nixosConfigurations.web1 = nixpkgs.lib.nixosSystem {
    modules = [
      snowglobe.nixosModules.default
      {
        networking.hostName = "web-01";
        networking.domain = "example.com";
      }
    ];
  };
}
```

That address is also what your SSH config sees, so `Host` patterns must match it: `Host web-01.example.com` or `Host *.example.com` here, not `Host web1`. nix-darwin leaves `networking.domain` unset by default, so a Mac is usually reached by its bare `networking.hostName`.

If the address that resolves on your network differs from the host's name (a host known only by IP, or reachable under a different name), keep the NixOS names as they are and map them in your SSH config with `HostName`, as shown [below](#ssh-config-entries).

## Choose the SSH user

snow picks the user like this:

1. With `snow deploy --user deploy`, snow logs in as `deploy` everywhere: for the copy, activation, reboots and remote hooks. A user on the command line overrides `User` in your SSH config.
2. Without `--user`, snow passes the bare address and lets `ssh` decide. ssh uses `User` from your SSH config if a matching entry sets one.
3. If no entry sets a user, ssh uses your local user name, which is OpenSSH's default.

snow never adds `root@` on its own. For most fleets the cleanest setup is a `User deploy` line in your SSH config, so `snow deploy` and an interactive `ssh web1` agree on who logs in. The [deploy user guide](/snowglobe/deploy-user/) explains how to create that account.

## Choose the SSH port

By default ssh decides the port: `Port` from your SSH config, or 22. When a host listens elsewhere, you can record the port in the host's own configuration:

```nix
{
  services.openssh.ports = [ 2222 ];
  snowglobe.deploy.sshPort = 2222;
}
```

snow then passes `-p 2222` on every connection to that host, including the `nix copy`. A port on the command line takes precedence over `Port` in your SSH config. Keeping the port next to `services.openssh.ports` means the two cannot drift apart, and anyone who deploys from the flake gets it without editing their own SSH config.

## SSH config entries

Your SSH config is the main tool for everything else. A typical block for a small fleet:

```text
Host web1 web2 db1
  User deploy
  IdentityFile ~/.ssh/deploy_ed25519
  IdentitiesOnly yes

Host nas1
  HostName 192.0.2.1
  User deploy
  Port 2222
  IdentityFile ~/.ssh/deploy_ed25519

Host *.example.com
  User deploy
  IdentityFile ~/.ssh/deploy_ed25519
```

The options that matter most:

| Option | What it does for a deployment |
|---|---|
| `HostName` | The real address to dial. Use it when the host's NixOS name does not resolve, or resolves to the wrong place. |
| `User` | The account to log in as, when you do not pass `--user`. |
| `Port` | The port, unless the host sets `snowglobe.deploy.sshPort`. |
| `IdentityFile` | The private key to offer. |
| `IdentitiesOnly yes` | Offer only that key, not every key in your agent. Servers close the connection after a few failed keys, so an agent with many keys can otherwise fail before reaching the right one. |
| `ProxyJump` | Reach the host through a bastion. See [below](#reach-hosts-through-a-jump-host). |

Check an entry with `ssh -G`, which prints the configuration ssh would use without connecting:

```bash
ssh -G web1 | grep -E '^(hostname|user|port|identityfile) '
```

## No prompts: batch mode

Every ssh command snow runs uses `-o BatchMode=yes`, so ssh never stops to ask for a password, a key passphrase or a host key confirmation. A deployment runs many connections in parallel, often unattended, and a prompt there would hang a host or be answered by nobody. With batch mode, anything that would prompt fails immediately with an error you can see in the report.

This means you need:

- **Key authentication.** Password logins cannot work. Load passphrase-protected keys into an agent before you deploy (`ssh-add ~/.ssh/deploy_ed25519`), or use a key without a passphrase where that fits your threat model, such as a CI runner.
- **Host keys in `known_hosts`.** An unknown host key fails with `Host key verification failed.` Connect once interactively and accept the key after checking its fingerprint, or add keys ahead of time:

  ```bash
  ssh-keyscan -t ed25519 web1 web2 db1 >> ~/.ssh/known_hosts
  ```

  `ssh-keyscan` does not verify what it fetches, so compare the fingerprints against the hosts (`ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub` on each) if the network between you is not trusted. A host that was reinstalled presents a new key, and ssh refuses it until you remove the old entry with `ssh-keygen -R web1`.

snow also sets a `ConnectTimeout` (10 seconds for most connections, 5 for reboot detection and reboots), so a host that does not answer fails quickly instead of waiting for the operating system's TCP timeout. Short commands, such as reading which system a host runs, use keepalives (`ServerAliveInterval=15`, `ServerAliveCountMax=3`), give up on a host that goes silent for about 45 seconds, and are limited to a minute in total. Activation and hooks do not use keepalives, because an activation can restart the host's network, and cutting the session at that point could kill the activation halfway.

A host that is unreachable fails on its own; the other hosts in the phase carry on.

[![A host is off the network: ssh times out and only that host fails](/snowglobe/unreachable.svg)](https://asciinema.org/a/cX25pebSCFgr3uDE)

## The ssh-agent

snow never forwards your agent (`-o ForwardAgent=no` on every connection, which overrides your SSH config). A host you deploy to could be compromised, and with your agent forwarded it could use your keys to log in to every other host while the session is open.

Before it builds anything, snow checks the agent named by `SSH_AUTH_SOCK`, if that variable is set, by running `ssh-add -l` with a 10-second limit:

- If the agent answers, with or without keys, the deployment goes ahead.
- If the socket cannot be reached at all (a stale `SSH_AUTH_SOCK` from an old session, for example), snow logs a warning and goes ahead; ssh still tries your key files.
- If the agent accepts the connection but never answers, snow stops the deployment before it touches any host, with `The ssh-agent did not answer within 10s`. Every ssh call would hang on such an agent, and a hang during a copy has no time limit. Check the agent with `ssh-add -l`, then restart it or unset `SSH_AUTH_SOCK`.

Hardware-backed and app-based agents can take a few seconds to list keys; the 10-second limit allows for that.

[![The ssh-agent never answers, so the deployment stops at once](/snowglobe/hung-agent.svg)](https://asciinema.org/a/eOyxRZmJRZ1fQB7f)

## NIX_SSHOPTS

`nix copy` reads extra ssh options from the `NIX_SSHOPTS` environment variable. snow appends its own options (batch mode, timeouts, no agent forwarding, and the port) to whatever you set, and leaves the rest in place:

```bash
NIX_SSHOPTS="-i ~/.ssh/deploy_ed25519" snow deploy web1
```

Two things to keep in mind:

- `NIX_SSHOPTS` only affects the copy. Activation, reboots, planning and remote hooks are separate ssh calls that do not read it, so an option set only there leaves them without it, and they can fail where the copy succeeded. Put options in your SSH config so every connection uses them.
- ssh keeps the first value it sees for an `-o` option. Your `NIX_SSHOPTS` come first, so they win over snow's for the copy.

## Reach hosts through a jump host

Because snow uses the system `ssh` and your config, a bastion works with `ProxyJump`:

```text
Host bastion
  HostName bastion.example.com
  User deploy
  IdentityFile ~/.ssh/deploy_ed25519

Host db1 k8s-1 k8s-2 k8s-3
  ProxyJump bastion
  User deploy
  IdentityFile ~/.ssh/deploy_ed25519
```

With `ProxyJump`, the bastion only forwards the TCP connection, and your machine authenticates to the final host through it. That is why it works even though snow disables agent forwarding. The bastion needs the same treatment as any host: key authentication without prompts, and its host key in your `known_hosts`.

The copy travels through the bastion too, so large closures move at the speed of that hop.

## Check every host

Before your first deployment, or after changing your SSH config, confirm that each host accepts a non-interactive login as the user snow will use:

```bash
for host in web1 web2 db1; do
  ssh -o BatchMode=yes -o ConnectTimeout=10 "$host" true && echo "$host ok"
done
```

Use the addresses snow connects to (the hostName and domain, as described [above](#the-address-snow-connects-to)). Each host should print `ok` without asking anything. Then `snow deploy --dry-run` builds, copies and plans every host without activating, which exercises the copy and the planning connections as well. Two caveats: a host that already runs the system you built is reported as unchanged before the copy (add `--redeploy` to go through the copy anyway), and `--dry-run` refuses to start if the selection includes a nix-darwin host, because nix-darwin can only switch; leave Macs out of the dry run by naming the other hosts.

When a connection fails during a deployment, the report shows the ssh command and its error. [Troubleshooting](/snowglobe/troubleshooting/) lists the common ones.
