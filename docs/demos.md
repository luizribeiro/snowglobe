# Demos

Recorded by the NixOS VM tests in `nix/e2e`: real pushes and rebuilds
between VMs. Click a demo to open it in the asciinema player.

## Three hosts in two phases switch to a new system

`snow push`

[![switch](demos/switch.svg)](https://asciinema.org/a/FG9AARHrB4eEO1jq)

## A host that must reboot is set to boot the new system, then rebooted

`snow push boron`

[![reboot](demos/reboot.svg)](https://asciinema.org/a/C3DbCPc7hX4bzUiW)

## One host fails to activate while the others succeed

`snow push`

[![activation-failure](demos/activation-failure.svg)](https://asciinema.org/a/Dr9UFyXxR6QaisGD)

## A pre-deploy hook fails, so its host is never activated

`snow push`

[![hook-failure](demos/hook-failure.svg)](https://asciinema.org/a/UFNK1GMXPRvTlx2x)

## A local hook's program is missing, so the push stops before building

`snow push`

[![missing-hook](demos/missing-hook.svg)](https://asciinema.org/a/P1HS1uDFAyp93jZz)

## A host is off the network: ssh times out and only that host fails

`snow push`

[![unreachable](demos/unreachable.svg)](https://asciinema.org/a/0dfwzXn7WFbCct6d)

## The ssh-agent never answers, so the push stops at once

`snow push`

[![hung-agent](demos/hung-agent.svg)](https://asciinema.org/a/P1luWXnv7DrreurJ)

## Two hosts in a serial group take turns

`snow push`

[![serial-group](demos/serial-group.svg)](https://asciinema.org/a/6mFMCpWYPMuoNSfW)

## A rebooted host never comes back before the deadline

`snow push --reboot-timeout 20`

[![reboot-deadline](demos/reboot-deadline.svg)](https://asciinema.org/a/6DeUKKMHWI0L41QX)

## snow rebuild warns before switching without a reboot, and is cancelled

`snow rebuild`

[![rebuild-cancel](demos/rebuild-cancel.svg)](https://asciinema.org/a/hFQFj2DXHULgLAUS)

## snow rebuild sets the new system to start on the next boot

`snow rebuild`

[![rebuild-boot](demos/rebuild-boot.svg)](https://asciinema.org/a/wAGeTKTHN0fkqnLE)

## snow rebuild reboots into the new system

`snow rebuild`

[![rebuild-reboot](demos/rebuild-reboot.svg)](https://asciinema.org/a/7Tlyl2BUHoinFGZK)
