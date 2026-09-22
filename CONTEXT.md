# GenOS persistent data

This glossary distinguishes what applications were told from what storage may
contain after interruption.

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
