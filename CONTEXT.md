# GenOS contracts

This glossary distinguishes authority, evidence and what applications were told
from what storage may contain after interruption.

## Language

**Persistent namespace**: User files and directories intended to survive a new
boot. In GenOS this is the `/USER` namespace.

**Session namespace**: Temporary user data belonging only to the current boot.
In GenOS this is the `/TMP` namespace.

**Acknowledged snapshot**: The complete persistent state most recently reported
as successfully saved to an application.
_Avoid_: Latest state, durable state when the storage assumptions are unstated.

**Recovered snapshot**: The complete persistent state selected when a volume is
opened after interruption. It can contain a mutation whose caller received no
successful acknowledgement.

**Quarantined volume**: A volume whose save outcome is uncertain or failed and
whose current mount refuses further persistent mutations until recovery.

**Console owner**: The process currently authorized to receive interactive
keyboard input and maintain the pending command line.
_Avoid_: Any process waiting for a keyboard event.

**Reference profile**: A named set of machine, tool, firmware and device
assumptions to which a verification result applies.
_Avoid_: All x86_64 machines, every QEMU configuration.

**Host stop**: Termination of the virtual machine by its supervising host,
without an ordered guest shutdown or a new durability acknowledgement.
_Avoid_: Guest shutdown.

**Guest shutdown**: A guest-initiated, ordered stop of services and device
activity before machine power-off.
_Avoid_: Closing the emulator window.
