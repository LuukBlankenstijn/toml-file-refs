#![doc = include_str!("../README.md")]

use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};
use toml::{Table, Value};

pub struct ConfigLoader {
    file_suffix: String,
    file_contents_suffix: String,
}

impl Default for ConfigLoader {
    fn default() -> Self {
        Self {
            file_suffix: String::from("_file"),
            file_contents_suffix: String::from("_file_contents"),
        }
    }
}

#[derive(Clone, Copy)]
enum FileRead {
    SingleLine,
    Contents,
}

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("failed to read config {path}: {source}")]
    ReadConfig {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid toml in {path}:\n{source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("{key}: failed to read {file}: {source}")]
    ReadRef {
        key: String,
        file: PathBuf,
        source: std::io::Error,
    },
    #[error("{key}: {file} contains more than one line")]
    MultiLine { key: String, file: PathBuf },
    #[error("{key}: expected a file path string, got {ty}")]
    NotAString { key: String, ty: &'static str },
    #[error("{key} is set more than once (directly or via file refs)")]
    Conflict { key: String },
    #[error("{path}: {source}")]
    Deserialize {
        path: String,
        source: toml::de::Error,
    },
}

impl ConfigLoader {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub const fn with_suffix(file_suffix: String, file_contents_suffix: String) -> Self {
        Self {
            file_suffix,
            file_contents_suffix,
        }
    }

    pub fn load<T: DeserializeOwned>(&self, path: impl AsRef<Path>) -> Result<T, LoadError> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|source| LoadError::ReadConfig {
            path: path.to_owned(),
            source,
        })?;
        let table: Table = text.parse().map_err(|source| LoadError::Parse {
            path: path.to_owned(),
            source,
        })?;

        let base = path.parent().unwrap_or(Path::new(""));
        let table = self.walk_table(table, base, "")?;

        serde_path_to_error::deserialize(Value::Table(table)).map_err(|e| LoadError::Deserialize {
            path: e.path().to_string(),
            source: e.into_inner(),
        })
    }

    fn walk_table(&self, table: Table, base: &Path, prefix: &str) -> Result<Table, LoadError> {
        let mut out = Table::new();

        for (key, value) in table {
            let key_path = join_key(prefix, &key);
            let file_ref = [
                (&self.file_suffix, FileRead::SingleLine),
                (&self.file_contents_suffix, FileRead::Contents),
            ]
            .into_iter()
            .filter_map(|(suffix, read)| {
                key.strip_suffix(suffix.as_str())
                    .filter(|stem| !stem.is_empty())
                    .map(|stem| (stem.to_owned(), read))
            })
            .min_by_key(|(stem, _)| stem.len());

            let (key, value) = match (file_ref, value) {
                (Some((stem, read)), Value::String(file)) => {
                    let file = base.join(file);
                    let contents = match read {
                        FileRead::SingleLine => read_single_line(&file, &key_path)?,
                        FileRead::Contents => read_contents(&file, &key_path)?,
                    };
                    (stem, Value::String(contents))
                }
                (Some(_), value) => {
                    return Err(LoadError::NotAString {
                        key: key_path,
                        ty: value.type_str(),
                    });
                }
                (None, Value::Table(t)) => {
                    (key, Value::Table(self.walk_table(t, base, &key_path)?))
                }
                (None, Value::Array(items)) => {
                    (key, Value::Array(self.walk_array(items, base, &key_path)?))
                }
                (None, value) => (key, value),
            };

            if out.contains_key(&key) {
                return Err(LoadError::Conflict {
                    key: join_key(prefix, &key),
                });
            }
            out.insert(key, value);
        }

        Ok(out)
    }

    fn walk_array(
        &self,
        items: Vec<Value>,
        base: &Path,
        prefix: &str,
    ) -> Result<Vec<Value>, LoadError> {
        items
            .into_iter()
            .enumerate()
            .map(|(i, item)| match item {
                Value::Table(t) => self
                    .walk_table(t, base, &format!("{prefix}[{i}]"))
                    .map(Value::Table),
                item => Ok(item),
            })
            .collect()
    }
}

fn join_key(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_owned()
    } else {
        format!("{prefix}.{key}")
    }
}

fn read_contents(file: &Path, key: &str) -> Result<String, LoadError> {
    std::fs::read_to_string(file).map_err(|source| LoadError::ReadRef {
        key: key.to_owned(),
        file: file.to_owned(),
        source,
    })
}

fn read_single_line(file: &Path, key: &str) -> Result<String, LoadError> {
    let mut s = read_contents(file, key)?;

    if s.ends_with('\n') {
        s.pop();
        if s.ends_with('\r') {
            s.pop();
        }
    }

    if s.contains('\n') {
        return Err(LoadError::MultiLine {
            key: key.to_owned(),
            file: file.to_owned(),
        });
    }

    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use tempfile::TempDir;

    #[derive(Debug, Deserialize, PartialEq)]
    struct Db {
        user: String,
        password: String,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct Upstream {
        name: String,
        token: String,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct App {
        db: Db,
        upstreams: Vec<Upstream>,
    }

    fn setup(config: &str, files: &[(&str, &str)]) -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        for (name, contents) in files {
            let path = dir.path().join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
        }
        let config_path = dir.path().join("config.toml");
        std::fs::write(&config_path, config).unwrap();
        (dir, config_path)
    }

    fn load_table(config: &str, files: &[(&str, &str)]) -> Result<Table, LoadError> {
        let (_dir, path) = setup(config, files);
        ConfigLoader::default().load(&path)
    }

    #[test]
    fn resolves_file_refs_in_nested_tables_and_arrays() {
        let (_dir, path) = setup(
            r#"
            [db]
            user = "admin"
            password_file = "secrets/db"

            [[upstreams]]
            name = "a"
            token_file = "secrets/a"

            [[upstreams]]
            name = "b"
            token = "inline"
            "#,
            &[("secrets/db", "hunter2\n"), ("secrets/a", "tok-a")],
        );

        let app: App = ConfigLoader::default().load(&path).unwrap();

        assert_eq!(
            app,
            App {
                db: Db {
                    user: "admin".into(),
                    password: "hunter2".into(),
                },
                upstreams: vec![
                    Upstream {
                        name: "a".into(),
                        token: "tok-a".into(),
                    },
                    Upstream {
                        name: "b".into(),
                        token: "inline".into(),
                    },
                ],
            }
        );
    }

    #[test]
    fn strips_exactly_one_trailing_line_ending() {
        let table = load_table(
            r#"
            lf_file = "lf"
            crlf_file = "crlf"
            cr_only_file = "cr"
            empty_file = "empty"
            "#,
            &[
                ("lf", "a\n"),
                ("crlf", "b\r\n"),
                ("cr", "c\r"),
                ("empty", "\n"),
            ],
        )
        .unwrap();

        assert_eq!(table["lf"].as_str(), Some("a"));
        assert_eq!(table["crlf"].as_str(), Some("b"));
        assert_eq!(table["cr_only"].as_str(), Some("c\r"));
        assert_eq!(table["empty"].as_str(), Some(""));
    }

    #[test]
    fn rejects_multi_line_ref_including_trailing_blank_line() {
        for contents in ["a\nb", "a\n\n"] {
            let err = load_table("[db]\npassword_file = \"secret\"", &[("secret", contents)])
                .unwrap_err();

            assert!(
                matches!(&err, LoadError::MultiLine { key, .. } if key == "db.password_file"),
                "{contents:?}: {err:?}"
            );
        }
    }

    #[test]
    fn missing_ref_file_reports_key_path() {
        let err = load_table("[[items]]\n[[items]]\ntoken_file = \"nope\"", &[]).unwrap_err();

        assert!(
            matches!(&err, LoadError::ReadRef { key, .. } if key == "items[1].token_file"),
            "{err:?}"
        );
    }

    #[test]
    fn non_string_file_ref_is_rejected() {
        let err = load_table("[db]\nport_file = 5432", &[]).unwrap_err();

        assert!(
            matches!(&err, LoadError::NotAString { key, ty } if key == "db.port_file" && *ty == "integer"),
            "{err:?}"
        );
    }

    #[test]
    fn direct_value_and_file_ref_conflict() {
        let err = load_table(
            "[db]\npassword = \"x\"\npassword_file = \"secret\"",
            &[("secret", "y")],
        )
        .unwrap_err();

        assert!(
            matches!(&err, LoadError::Conflict { key } if key == "db.password"),
            "{err:?}"
        );
    }

    #[test]
    fn bare_suffix_key_is_kept_verbatim() {
        let table = load_table(r#"_file = "not-a-ref""#, &[]).unwrap();

        assert_eq!(table["_file"].as_str(), Some("not-a-ref"));
    }

    #[test]
    fn custom_suffix_replaces_default() {
        let (_dir, path) = setup(
            r#"
            token_path = "secret"
            other_file = "literal"
            "#,
            &[("secret", "s3cr3t")],
        );

        let table: Table = ConfigLoader::with_suffix("_path".into(), "_path_contents".into())
            .load(&path)
            .unwrap();

        assert_eq!(table["token"].as_str(), Some("s3cr3t"));
        assert_eq!(table["other_file"].as_str(), Some("literal"));
    }

    #[test]
    fn contents_suffix_reads_whole_file_verbatim() {
        let table = load_table(
            r#"
            [tls]
            cert_file_contents = "cert.pem"
            "#,
            &[("cert.pem", "-----BEGIN-----\nabc\n-----END-----\n")],
        )
        .unwrap();

        assert_eq!(
            table["tls"]["cert"].as_str(),
            Some("-----BEGIN-----\nabc\n-----END-----\n")
        );
    }

    #[test]
    fn longest_matching_suffix_wins() {
        let (_dir, path) = setup(
            r#"
            cert_full_file = "cert"
            token_file = "token"
            "#,
            &[("cert", "a\nb\n"), ("token", "t\n")],
        );

        let table: Table = ConfigLoader::with_suffix("_file".into(), "_full_file".into())
            .load(&path)
            .unwrap();

        assert_eq!(table["cert"].as_str(), Some("a\nb\n"));
        assert_eq!(table["token"].as_str(), Some("t"));
    }

    #[test]
    fn contents_ref_conflicts_with_line_ref_and_rejects_non_strings() {
        let conflict = load_table(
            "password_file = \"a\"\npassword_file_contents = \"a\"",
            &[("a", "x")],
        )
        .unwrap_err();
        let not_string = load_table("port_file_contents = 1", &[]).unwrap_err();

        assert!(
            matches!(&conflict, LoadError::Conflict { key } if key == "password"),
            "{conflict:?}"
        );
        assert!(
            matches!(&not_string, LoadError::NotAString { key, ty } if key == "port_file_contents" && *ty == "integer"),
            "{not_string:?}"
        );
    }

    #[test]
    fn deserialize_error_reports_field_path() {
        let (_dir, path) = setup(
            r#"
            [db]
            user = "admin"
            password = "x"

            [[upstreams]]
            name = "a"
            token = 1
            "#,
            &[],
        );

        let err = ConfigLoader::default().load::<App>(&path).unwrap_err();

        assert!(
            matches!(&err, LoadError::Deserialize { path, .. } if path == "upstreams[0].token"),
            "{err:?}"
        );
    }

    #[test]
    fn invalid_toml_and_missing_config_are_distinguished() {
        let parse = load_table("not = [valid", &[]).unwrap_err();
        let missing = ConfigLoader::default()
            .load::<Table>(Path::new("/nonexistent/config.toml"))
            .unwrap_err();

        assert!(matches!(parse, LoadError::Parse { .. }), "{parse:?}");
        assert!(
            matches!(missing, LoadError::ReadConfig { .. }),
            "{missing:?}"
        );
    }
}
