# Demos

Recorded by the NixOS VM tests in `nix/e2e`: real pushes and rebuilds
between VMs. Click a demo to open it in the asciinema player.

## Three hosts in two phases switch to a new system

`snow push`

[![switch](demos/switch.svg)](https://asciinema.org/a/6J8p7HDK0Df1tPSx)

## A host that must reboot is set to boot the new system, then rebooted

`snow push boron`

[![reboot](demos/reboot.svg)](https://asciinema.org/a/CRvXGdOvXPBkkiDV)

## snow push --boot sets the new system for the next boot, without rebooting

`snow push --boot boron`

[![push-boot](demos/push-boot.svg)](https://asciinema.org/a/8BSwYPCHAGjHvwxO)

## One host fails to activate while the others succeed

`snow push`

[![activation-failure](demos/activation-failure.svg)](https://asciinema.org/a/NRfbtSt4AyH53Xsy)

## A pre-deploy hook fails, so its host is never activated

`snow push`

[![hook-failure](demos/hook-failure.svg)](https://asciinema.org/a/SjhKcts8A6EwwRoG)

## A local hook's program is missing, so the push stops before building

`snow push`

[![missing-hook](demos/missing-hook.svg)](https://asciinema.org/a/uH01eTmwST7t8kNE)

## A host is off the network: ssh times out and only that host fails

`snow push`

[![unreachable](demos/unreachable.svg)](https://asciinema.org/a/cX25pebSCFgr3uDE)

## The ssh-agent never answers, so the push stops at once

`snow push`

[![hung-agent](demos/hung-agent.svg)](https://asciinema.org/a/eOyxRZmJRZ1fQB7f)

## Two hosts in a serial group take turns

`snow push`

[![serial-group](demos/serial-group.svg)](https://asciinema.org/a/DfWYNZ7K7KCfiFvg)

## A rebooted host never comes back before the deadline

`snow push --reboot-timeout 20`

[![reboot-deadline](demos/reboot-deadline.svg)](https://asciinema.org/a/eFy1uI5b9rnfI0jt)

## snow rebuild warns before switching without a reboot, and is cancelled

`snow rebuild`

[![rebuild-cancel](demos/rebuild-cancel.svg)](https://asciinema.org/a/BCTneCUsUzcuTWXt)

## snow rebuild sets the new system to start on the next boot

`snow rebuild`

[![rebuild-boot](demos/rebuild-boot.svg)](https://asciinema.org/a/6D3s0ETGu3gsEkXD)

## snow rebuild reboots into the new system

`snow rebuild`

[![rebuild-reboot](demos/rebuild-reboot.svg)](https://asciinema.org/a/HrREagz2DZPhuXDC)

## snow rebuild asks for the sudo password once, before it starts

`snow rebuild`

[![rebuild-sudo](demos/rebuild-sudo.svg)](https://asciinema.org/a/n6O5yoNHWIld5ZX2)

## snow rebuild keeps sudo from forgetting the password during a long build

`snow rebuild`

[![rebuild-sudo-keepalive](demos/rebuild-sudo-keepalive.svg)](https://asciinema.org/a/CCKjX20YntNFIMFC)
