# Demos

Every clip below is one of the NixOS VM tests in `nix/e2e`. Real deployments
and rebuilds between VMs, and the test checks they worked, so what you see
here is tested.

## snow list shows web1, with no phase or tags yet

`snow list`

![tutorial-list](demos/tutorial-list.svg)

## snow deploy --dry-run previews the change to web1

`snow deploy --dry-run web1`

![tutorial-dry-run](demos/tutorial-dry-run.svg)

## snow deploy switches web1 to its new system

`snow deploy web1`

![tutorial-deploy](demos/tutorial-deploy.svg)

## web1 already runs its new system, so it is left alone

`snow deploy web1`

![tutorial-unchanged](demos/tutorial-unchanged.svg)

## web1 and web2 deploy in two phases

`snow list; snow deploy`

![tutorial-phases](demos/tutorial-phases.svg)

## Twelve hosts in four phases switch to a new system

`snow deploy`

![switch](demos/switch.svg)

## A host that must reboot is set to boot the new system, then rebooted

`snow deploy web1`

![reboot](demos/reboot.svg)

## snow deploy --boot sets the new system for the next boot, without rebooting

`snow deploy --boot db1`

![deploy-boot](demos/deploy-boot.svg)

## One host fails to activate, so the next phase is skipped

`snow deploy`

![activation-failure](demos/activation-failure.svg)

## A pre-deploy hook fails, so its host is never activated

`snow deploy`

![hook-failure](demos/hook-failure.svg)

## A local hook's program is missing, so the deployment stops before building

`snow deploy`

![missing-hook](demos/missing-hook.svg)

## A host is off the network: ssh times out and only that host fails

`snow deploy`

![unreachable](demos/unreachable.svg)

## The ssh-agent never answers, so the deployment stops at once

`snow deploy`

![hung-agent](demos/hung-agent.svg)

## Two hosts in a serial group take turns

`snow deploy`

![serial-group](demos/serial-group.svg)

## A rebooted host never comes back before the deadline

`snow deploy --reboot-timeout 20`

![reboot-deadline](demos/reboot-deadline.svg)

## snow rebuild warns before switching without a reboot, and is cancelled

`snow rebuild`

![rebuild-cancel](demos/rebuild-cancel.svg)

## snow rebuild sets the new system to start on the next boot

`snow rebuild`

![rebuild-boot](demos/rebuild-boot.svg)

## snow rebuild reboots into the new system

`snow rebuild`

![rebuild-reboot](demos/rebuild-reboot.svg)

## snow rebuild asks for the sudo password once, before it starts

`snow rebuild`

![rebuild-sudo](demos/rebuild-sudo.svg)

## snow rebuild keeps sudo from forgetting the password during a long build

`snow rebuild`

![rebuild-sudo-keepalive](demos/rebuild-sudo-keepalive.svg)
