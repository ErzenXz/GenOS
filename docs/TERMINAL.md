# GenOS terminal

The active interface is the Ring 3 serial shell. Normal builds keep development
syscall/keystroke tracing out of the interactive session. Startup and failure
messages remain available; `validation-boot` retains the detailed proof trace.
Start with `make run`. This serial-only QEMU session does not capture the mouse.
To quit the emulator, press Control+A, release both keys, then press `x`. These
are host emulator controls, not a guest shutdown command.
Type one command and press Enter. Commands use plain arguments, not shell quoting,
pipelines or redirection. The current namespace is rooted at `/` with no `cd`.

| Command | Behavior |
| --- | --- |
| `help` | Bounded usage lines for every available command |
| `clear` | Clear the serial screen and move the cursor home |
| `uname` | Version, architecture and experimental ABI |
| `echo TEXT` | Print text |
| `net` | Report whether network configuration is available; no new ping |
| `mem` | Read allocator counters and consistency from `/MEMORY.STATUS` |
| `ls [PATH]` | List `/` or the specified directory |
| `cat PATH` | Read a file |
| `stat PATH` | File information |
| `touch PATH` | Create a missing file |
| `write PATH TEXT` | Replace file contents |
| `append PATH TEXT` | Append exact text, without an automatic separator |
| `mkdir PATH` | Create a directory |
| `rm PATH` | Remove a file or empty directory |
| `run init` | Start the built-in example process |
| `run init hold` | Keep the example running for lifecycle testing |
| `ps` | Show jobs owned by this shell |
| `kill JOB` | Stop the named shell job |
| `wait JOB` | Reap a finished job; report if it is still running |

Writable files belong below `/USER/`. The current file budget is 512 bytes and
the pathname budget is 64 bytes. Paths are case-insensitive and accept ASCII
letters, digits, `.`, `_` and `-` within components. Dot-navigation components,
duplicate separators, trailing separators and spaces in names are rejected.
`/MEMORY.STATUS` is reserved and read-only. Job numbers come from `run`/`ps`, not
the task ID or runtime PID.

## A small end-to-end check

```text
help
mem
write /USER/TEST.TXT hello
cat /USER/TEST.TXT
run init hold
mem
ps
kill 1
wait 1
mem
clear
echo terminal ready
```

Use the actual printed job number if it is not 1. Memory usage should rise while
the example is live and return after teardown. `mem` counts managed physical
frames, including page tables; it is not total hardware memory or a per-process
memory profiler. See [the memory contract](MEMORY.md).

## Scope

This remains an experimental console OS, not a complete everyday shell. General
program launch, working-directory navigation, quoting, pipelines, redirection,
interactive job control, a mature terminal escape parser and a supported shutdown
interface remain future work. The graphical UI remains deferred behind foundation
correctness. Familiar command names describe their documented operations; they do
not imply Linux/POSIX compatibility.

Normal acceptance uses these same visible echoes and outputs to check help,
network configuration, read-only memory diagnostics, canonical paths, process
cleanup, persistent file I/O and clear/redraw behavior. It rejects development
trace leakage and missing or out-of-order responses. The separate validation suite
continues to prove the lower-level mechanisms with detailed serial evidence.
