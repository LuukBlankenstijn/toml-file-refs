# toml-file-refs

Load a TOML config file into any `serde` type, where any key ending in `_file`
is replaced by the contents of the file it points to.

This lets you keep secrets out of your config and point to files from Docker
secrets, systemd credentials, agenix, sops-nix, Kubernetes secret mounts, etc.:

```toml
[db]
user = "admin"
password_file = "/run/secrets/db_password"

[tls]
cert_file_contents = "certs/server.pem"
```

```rust,no_run
use serde::Deserialize;
use toml_file_refs::ConfigLoader;

#[derive(Deserialize)]
struct Config {
    db: Db,
    tls: Tls,
}

#[derive(Deserialize)]
struct Db {
    user: String,
    password: String,
}

#[derive(Deserialize)]
struct Tls {
    cert: String,
}

fn main() -> Result<(), toml_file_refs::LoadError> {
    let config: Config = ConfigLoader::new().load("config.toml")?;
    println!("connecting as {}", config.db.user);
    Ok(())
}
```

Your structs only use the plain field names (`password`, `cert`). Whether a value
is written inline or comes from a file is decided in the config file alone.

## Suffixes

| Key                   | Becomes | Value                                                                                    |
| --------------------- | ------- | ---------------------------------------------------------------------------------------- |
| `<key>_file`          | `<key>` | A single line. One trailing `\n` or `\r\n` is removed; any other line break is an error. |
| `<key>_file_contents` | `<key>` | The whole file, verbatim (trailing newline included).                                    |

Rules:

- Relative paths are resolved against the directory of the config file.
  Absolute paths are used as-is.
- References are resolved in nested tables and in arrays of tables.
- The value of a reference key must be a string.
- Setting a key more than once (e.g. `password` and `password_file`, or
  `password_file` and `password_file_contents`) is an error.
- A key that is exactly the suffix (`_file = "..."`) is kept as a normal key.

## Custom suffixes

```rust
use toml_file_refs::ConfigLoader;

let loader = ConfigLoader::with_suffix("_path".into(), "_path_contents".into());
```

When a key matches both suffixes, the longer one wins.

## Errors

`LoadError` tells apart a config file that can't be read, invalid TOML, a
referenced file that can't be read, a multi-line `_file` value, a non-string
reference, a duplicate key, and a deserialization failure. Errors about a
specific entry include its full key path (e.g. `upstreams[1].token_file`).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
