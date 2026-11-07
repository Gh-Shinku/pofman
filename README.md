# pofman

A lightweight SSH port forwarding manager.

Pofman organizes SSH port forwarding configurations into profiles, each consisting of a `name` field and a `command` field.

By default, Pofman reads its configuration from `~/.config/pofman/config.toml`. You can specify a custom configuration file using the `POFMAN_CONFIG` environment variable.

Given the simplicity of the configuration format, Pofman intentionally leaves profile management to the user.

Here is an example configuration:

```toml
[[profiles]]
name = "database"
command = "ssh -L 127.0.0.1:5433:127.0.0.1:5432 user@server"

[[profiles]]
name = "proxy"
command = "ssh -D 127.0.0.1:1080 user@server"
```

Once configured, simply run `pofman` to manage your profiles.

The CLI provides the following commands:

```text
A lightweight SSH port forwarding manager

Usage: pofman.exe [COMMAND]

Commands:
  list   List configured profiles and running processes
  start  Start a profile in the background
  stop   Stop a managed profile
  help   Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```
