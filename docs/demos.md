# Demos

Every clip below is one of the NixOS VM tests in `nix/e2e`. Real deployments and
rebuilds between VMs, and the test checks they worked, so what you see here
is tested. Click a demo to open it in the asciinema player.

## Three hosts in two phases switch to a new system

`snow deploy`

[![switch](demos/switch.svg)](https://asciinema.org/a/6J8p7HDK0Df1tPSx)

## A host that must reboot is set to boot the new system, then rebooted

`snow deploy boron`

[![reboot](demos/reboot.svg)](https://asciinema.org/a/CRvXGdOvXPBkkiDV)

## snow deploy --boot sets the new system for the next boot, without rebooting

`snow deploy --boot boron`

[![deploy-boot](demos/deploy-boot.svg)](https://asciinema.org/a/8BSwYPCHAGjHvwxO)

## One host fails to activate while the others succeed

`snow deploy`

[![activation-failure](demos/activation-failure.svg)](https://asciinema.org/a/NRfbtSt4AyH53Xsy)

## A pre-deploy hook fails, so its host is never activated

`snow deploy`

[![hook-failure](demos/hook-failure.svg)](https://asciinema.org/a/SjhKcts8A6EwwRoG)

## A local hook's program is missing, so the deployment stops before building

`snow deploy`

[![missing-hook](demos/missing-hook.svg)](https://asciinema.org/a/uH01eTmwST7t8kNE)

## A host is off the network: ssh times out and only that host fails

`snow deploy`

[![unreachable](demos/unreachable.svg)](https://asciinema.org/a/cX25pebSCFgr3uDE)

## The ssh-agent never answers, so the deployment stops at once

`snow deploy`

[![hung-agent](demos/hung-agent.svg)](https://asciinema.org/a/eOyxRZmJRZ1fQB7f)

## Two hosts in a serial group take turns

`snow deploy`

[![serial-group](demos/serial-group.svg)](https://asciinema.org/a/DfWYNZ7K7KCfiFvg)

## A rebooted host never comes back before the deadline

`snow deploy --reboot-timeout 20`

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
