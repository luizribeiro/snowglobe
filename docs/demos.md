# Demos

Recorded by the NixOS VM tests in `nix/e2e`: real pushes and rebuilds
between VMs. Click a demo to open it in the asciinema player.

## Three hosts in two phases switch to a new system

`snow push`

[![switch](demos/switch.svg)](https://asciinema.org/a/PkCZeHp10QEiaCpJ)

## A host that must reboot is set to boot the new system, then rebooted

`snow push boron`

[![reboot](demos/reboot.svg)](https://asciinema.org/a/lczqSfBlIJKTMVHf)

## One host fails to activate while the others succeed

`snow push`

[![activation-failure](demos/activation-failure.svg)](https://asciinema.org/a/oTXsv1GvFY2i04yp)

## A pre-deploy hook fails, so its host is never activated

`snow push`

[![hook-failure](demos/hook-failure.svg)](https://asciinema.org/a/9OD1MmUv8kv9g5ju)

## A local hook's program is missing, so the push stops before building

`snow push`

[![missing-hook](demos/missing-hook.svg)](https://asciinema.org/a/wmNd2gUnFbvJHOY6)

## A host is off the network: ssh times out and only that host fails

`snow push`

[![unreachable](demos/unreachable.svg)](https://asciinema.org/a/O6sqtapOROsYMVwd)

## The ssh-agent never answers, so the push stops at once

`snow push`

[![hung-agent](demos/hung-agent.svg)](https://asciinema.org/a/dGZA6FO2nUaQXmxP)

## Two hosts in a serial group take turns

`snow push`

[![serial-group](demos/serial-group.svg)](https://asciinema.org/a/8Ifc0ENK0XkTO7oR)

## A rebooted host never comes back before the deadline

`snow push --reboot-timeout 20`

[![reboot-deadline](demos/reboot-deadline.svg)](https://asciinema.org/a/pVRbHFt5bFjAcSsg)

## snow rebuild warns before switching without a reboot, and is cancelled

`snow rebuild`

[![rebuild-cancel](demos/rebuild-cancel.svg)](https://asciinema.org/a/cDZo95bg1xh6IjPU)

## snow rebuild sets the new system to start on the next boot

`snow rebuild`

[![rebuild-boot](demos/rebuild-boot.svg)](https://asciinema.org/a/WZlCgUmwiYue8snx)

## snow rebuild reboots into the new system

`snow rebuild`

[![rebuild-reboot](demos/rebuild-reboot.svg)](https://asciinema.org/a/ObgK3izhxp2nG9Hl)
