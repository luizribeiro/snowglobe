# Demos

Recorded by the NixOS VM tests in `nix/e2e`: real pushes and rebuilds
between VMs. Click a preview to play it.

## Three hosts in two phases switch to a new system

`snow push`

[![switch](https://asciinema.org/a/Z9UPbDMklRqdVFws.svg)](https://asciinema.org/a/Z9UPbDMklRqdVFws)

## A host that must reboot is set to boot the new system, then rebooted

`snow push boron`

[![reboot](https://asciinema.org/a/NngzPONPTS1KjO3W.svg)](https://asciinema.org/a/NngzPONPTS1KjO3W)

## One host fails to activate while the others succeed

`snow push`

[![activation-failure](https://asciinema.org/a/RzDf2opIj9zsY23c.svg)](https://asciinema.org/a/RzDf2opIj9zsY23c)

## A pre-deploy hook fails, so its host is never activated

`snow push`

[![hook-failure](https://asciinema.org/a/5BlYLFROSuhGjK06.svg)](https://asciinema.org/a/5BlYLFROSuhGjK06)

## A local hook's program is missing, so the push stops before building

`snow push`

[![missing-hook](https://asciinema.org/a/sY9BRvRGiUsooOJj.svg)](https://asciinema.org/a/sY9BRvRGiUsooOJj)

## A host is off the network: ssh times out and only that host fails

`snow push`

[![unreachable](https://asciinema.org/a/68DPWsx1GnRODHmO.svg)](https://asciinema.org/a/68DPWsx1GnRODHmO)

## The ssh-agent never answers, so the push stops at once

`snow push`

[![hung-agent](https://asciinema.org/a/QAY6JJ2iEn15J7pG.svg)](https://asciinema.org/a/QAY6JJ2iEn15J7pG)

## Two hosts in a serial group take turns

`snow push`

[![serial-group](https://asciinema.org/a/MbxHO4fpDxfThD6Q.svg)](https://asciinema.org/a/MbxHO4fpDxfThD6Q)

## A rebooted host never comes back before the deadline

`snow push --reboot-timeout 20`

[![reboot-deadline](https://asciinema.org/a/oNYrWADf6OdipBrV.svg)](https://asciinema.org/a/oNYrWADf6OdipBrV)

## snow rebuild warns before switching without a reboot, and is cancelled

`snow rebuild`

[![rebuild-cancel](https://asciinema.org/a/drpi6luHRvQPdcJw.svg)](https://asciinema.org/a/drpi6luHRvQPdcJw)

## snow rebuild sets the new system to start on the next boot

`snow rebuild`

[![rebuild-boot](https://asciinema.org/a/YnR0vkSBTLJAaKG7.svg)](https://asciinema.org/a/YnR0vkSBTLJAaKG7)

## snow rebuild reboots into the new system

`snow rebuild`

[![rebuild-reboot](https://asciinema.org/a/x2HNiouhw8EB0GF0.svg)](https://asciinema.org/a/x2HNiouhw8EB0GF0)
