# Demos

Recorded by the NixOS VM tests in `nix/e2e`: real pushes and rebuilds
between VMs. Click a demo to open it in the asciinema player.

## Three hosts in two phases switch to a new system

`snow push`

[![switch](demos/switch.svg)](https://asciinema.org/a/xUtYcB8o7ArSRG7y)

## A host that must reboot is set to boot the new system, then rebooted

`snow push boron`

[![reboot](demos/reboot.svg)](https://asciinema.org/a/ZEpZLLEApHx7wvnl)

## One host fails to activate while the others succeed

`snow push`

[![activation-failure](demos/activation-failure.svg)](https://asciinema.org/a/XhcGtzDfvNhLCdDp)

## A pre-deploy hook fails, so its host is never activated

`snow push`

[![hook-failure](demos/hook-failure.svg)](https://asciinema.org/a/4waHejr3uT7HdWgu)

## A local hook's program is missing, so the push stops before building

`snow push`

[![missing-hook](demos/missing-hook.svg)](https://asciinema.org/a/EcXZxfNoldX81TK2)

## A host is off the network: ssh times out and only that host fails

`snow push`

[![unreachable](demos/unreachable.svg)](https://asciinema.org/a/3OUD7KTVUikm5Yp3)

## The ssh-agent never answers, so the push stops at once

`snow push`

[![hung-agent](demos/hung-agent.svg)](https://asciinema.org/a/LbhLxq1GdufLS4nh)

## Two hosts in a serial group take turns

`snow push`

[![serial-group](demos/serial-group.svg)](https://asciinema.org/a/dsUyU0HPndU7UKI9)

## A rebooted host never comes back before the deadline

`snow push --reboot-timeout 20`

[![reboot-deadline](demos/reboot-deadline.svg)](https://asciinema.org/a/XYEyYIQ5dKg13Ek8)

## snow rebuild warns before switching without a reboot, and is cancelled

`snow rebuild`

[![rebuild-cancel](demos/rebuild-cancel.svg)](https://asciinema.org/a/ldXJvsGanVfIIBgJ)

## snow rebuild sets the new system to start on the next boot

`snow rebuild`

[![rebuild-boot](demos/rebuild-boot.svg)](https://asciinema.org/a/Ah8UUr53G2IjIzRi)

## snow rebuild reboots into the new system

`snow rebuild`

[![rebuild-reboot](demos/rebuild-reboot.svg)](https://asciinema.org/a/WFCAmud26dAGFCwa)
