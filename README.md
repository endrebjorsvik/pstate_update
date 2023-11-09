# pstate_update

[power-profiles-daemon][ppd] is a nice and simple daemon which controls the power states
in both Gnome and KDE. It supports several Linux power "drivers", like `platform_profile`,
`amd_pstate`, `intel_pstate`, etc. However, it can currently use only one of these
drivers, not multiple of them at the same time.

Some computers like ThinkPad T14 G3 AMD, exposes multiple of the available interfaces
(eg. both `platform_profiles` and `amd_pstate`). power-profiles-daemon will then only
control the `platform_profile`, while `amd_pstate` remains uncontrolled. It turns out
that `amd_pstate` Energy Performance Preference (EPP) remains at `performance` in my
case, which burns a lot of excess energy when I do not need it. To fix this, I wrote
this small daemon which does the following:

- Use the DBus interface for power-profiles-daemon and listen for `ActiveProfile`
  property changes.
- Translate the PPD power profile to a desired AMD PState EPP.
- Write the selected AMD PState EPP to the kernel `sysfs` interface. This is written
  on all available CPU cores/threads that are exposed.

It also comes with a conveniet systemd unit file to launch the service in the background.

[ppd]: https://gitlab.freedesktop.org/hadess/power-profiles-daemon
