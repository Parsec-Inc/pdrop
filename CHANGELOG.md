## v2.0.0 (2026-11-26)

### BREAKING CHANGE

- HELLO requires a memebership key, the relay and its clients must be updated

### Feat

- **relay**: configure and save several relays from the window
- relay interface support
- **relay.rs**: added config option on relay and room on cli
- dial and accepts
- **relay.rs**: added relay room
- **net.rs**: added p2p route
- basic relay features

### Fix

- **relay**: address second review round
- **relay.rs**: fixed windows token generation
- solved ghost member

## v1.2.4 (2026-11-23)

## v1.2.3 (2026-11-23)

## v1.2.2 (2026-11-23)

### Fix

- toast diagnostics
- added accept and refuse toast options

## v1.2.1 (2026-11-19)

### Fix

- layout remapping

## v1.2.0 (2026-11-19)

### Feat

- zstd compression over transfer
- added speed and ETA transfer data

### Fix

- merge conflict
- windows clipboard support patched

## v1.1.0 (2026-11-18)

### Feat

- added new views for pdrop

### Fix

- clippy diagnostics

## v1.0.1 (2026-11-17)

### Fix

- **net.rs**: removed buffer overload on socket ARP fail

## v1.0.0 (2026-11-17)

### BREAKING CHANGE

- the discovery beacon and the transfer header moved from
PDROP1 to PDROP3 and all traffic is now encrypted. Older versions cannot
see or talk to this one, so update every machine together.

### Feat

- **net**: encrypted transfers with device identity and pairing code

## v0.2.0 (2026-11-17)

### Feat

- **firewall**: aviso no app quando o firewall parece bloquear o pdrop, com o comando para liberar

## v0.1.2 (2026-11-17)

## v0.1.1 (2026-11-17)

## v0.1.0 (2026-11-17)

### Feat

- new frontend
- basic implementation
