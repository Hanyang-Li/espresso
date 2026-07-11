## Daemon

`espresso` keeps the Mac awake with a per-process idle-sleep assertion. To also
keep it awake with the lid closed (screen off, no sleep, even on battery), a small
root helper is required:

    sudo espresso daemon install     # one time
    espresso daemon status
    sudo espresso daemon uninstall

The helper is launched on demand by launchd and self-exits when no sessions remain.

### SleepDisabled is global

Lid-closed wake relies on the kernel `SleepDisabled` flag, which is a single global
switch with no owner. espresso sets it while any session is active and clears it when
the last one ends. This is last-writer-wins: it can override a `SleepDisabled` you set
manually via `pmset` or that another app set, and vice versa.
