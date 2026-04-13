use anyhow::{Context, Result};
use console::Term;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use crate::project::{
    SubmoduleProtocol,
    index::{DEFAULT_INDEX_FILENAME, DEFAULT_INDEX_URL},
    resolver::RepositoryRemoteBases,
};

pub const CPKG_HOME_DIRNAME: &str = ".cpkg";
pub const GLOBAL_CONFIG_FILENAME: &str = "config.toml";
pub const CURRENT_GLOBAL_CONFIG_FORMAT_VERSION: u32 = 1;
pub const DEFAULT_GLOBAL_ORG_SOURCE_NAME: &str = "wtr";

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GlobalConfig {
    #[serde(default = "default_format_version")]
    pub format_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_org: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub index: Vec<IndexSourceConfig>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub org: Vec<NamedOrgSource>,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            format_version: CURRENT_GLOBAL_CONFIG_FORMAT_VERSION,
            default_org: None,
            index: Vec::new(),
            org: Vec::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct IndexSourceConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_path: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NamedOrgSource {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_base: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub https_base: Option<String>,
    #[serde(default)]
    pub default_protocol: SubmoduleProtocol,
}

fn default_format_version() -> u32 {
    CURRENT_GLOBAL_CONFIG_FORMAT_VERSION
}

fn render_global_config(config: &GlobalConfig) -> Result<String> {
    toml::to_string_pretty(config).context("failed to serialize global cpkg config")
}

pub fn initialized_global_config_template() -> GlobalConfig {
    let defaults = RepositoryRemoteBases::wtr_default();
    GlobalConfig {
        format_version: CURRENT_GLOBAL_CONFIG_FORMAT_VERSION,
        default_org: Some(DEFAULT_GLOBAL_ORG_SOURCE_NAME.to_string()),
        index: vec![IndexSourceConfig {
            path: None,
            url: Some(DEFAULT_INDEX_URL.to_string()),
            cache_path: Some(DEFAULT_INDEX_FILENAME.to_string()),
        }],
        org: vec![NamedOrgSource {
            name: DEFAULT_GLOBAL_ORG_SOURCE_NAME.to_string(),
            ssh_base: defaults.ssh_base,
            https_base: defaults.https_base,
            default_protocol: SubmoduleProtocol::Ssh,
        }],
    }
}

fn validate_index_source(source: &IndexSourceConfig, label: &str) -> Result<()> {
    match (source.path.as_deref(), source.url.as_deref()) {
        (Some(_), Some(_)) => anyhow::bail!("{label} cannot set both `path` and `url`"),
        (None, None) => anyhow::bail!("{label} must set either `path` or `url`"),
        (Some(_), None) if source.cache_path.is_some() => {
            anyhow::bail!("{label} cannot set `cache_path` without `url`")
        }
        _ => Ok(()),
    }
}

fn validate_org_source(source: &NamedOrgSource, label: &str) -> Result<()> {
    if source.ssh_base.is_none() && source.https_base.is_none() {
        anyhow::bail!("{label} must set at least one of `ssh_base` or `https_base`");
    }

    match source.default_protocol {
        SubmoduleProtocol::Ssh if source.ssh_base.is_none() => {
            anyhow::bail!("{label} sets default protocol to `ssh` but does not define `ssh_base`")
        }
        SubmoduleProtocol::Https if source.https_base.is_none() => anyhow::bail!(
            "{label} sets default protocol to `https` but does not define `https_base`"
        ),
        _ => Ok(()),
    }
}

pub fn validate_global_config(config: &GlobalConfig) -> Result<()> {
    if config.format_version != CURRENT_GLOBAL_CONFIG_FORMAT_VERSION {
        anyhow::bail!(
            "unsupported global config format version {}",
            config.format_version
        );
    }

    for (index, source) in config.index.iter().enumerate() {
        validate_index_source(source, &format!("global [index] entry {}", index + 1))?;
    }

    let mut seen_names = BTreeSet::new();
    for source in &config.org {
        if !seen_names.insert(source.name.clone()) {
            anyhow::bail!("duplicate global org source '{}'", source.name);
        }
        validate_org_source(source, &format!("global [org.{}]", source.name))?;
    }

    if let Some(default_org) = config.default_org.as_deref() {
        if !config.org.iter().any(|source| source.name == default_org) {
            anyhow::bail!("global default org source '{}' is not defined", default_org);
        }
    }

    Ok(())
}

pub fn home_dir() -> Result<PathBuf> {
    home_dir_from_env(
        env::var_os("HOME"),
        env::var_os("USERPROFILE"),
        env::var_os("HOMEDRIVE"),
        env::var_os("HOMEPATH"),
    )
    .ok_or_else(|| anyhow::anyhow!("failed to resolve user home directory"))
}

pub(crate) fn home_dir_from_env(
    home: Option<OsString>,
    userprofile: Option<OsString>,
    homedrive: Option<OsString>,
    homepath: Option<OsString>,
) -> Option<PathBuf> {
    if let Some(home) = home {
        if !home.is_empty() {
            return Some(PathBuf::from(home));
        }
    }

    if let Some(userprofile) = userprofile {
        if !userprofile.is_empty() {
            return Some(PathBuf::from(userprofile));
        }
    }

    match (homedrive, homepath) {
        (Some(homedrive), Some(homepath)) if !homedrive.is_empty() && !homepath.is_empty() => {
            Some(PathBuf::from(format!(
                "{}{}",
                homedrive.to_string_lossy(),
                homepath.to_string_lossy()
            )))
        }
        _ => None,
    }
}

pub fn cpkg_home_dir() -> Result<PathBuf> {
    Ok(home_dir()?.join(CPKG_HOME_DIRNAME))
}

pub fn global_config_path() -> Result<PathBuf> {
    Ok(cpkg_home_dir()?.join(GLOBAL_CONFIG_FILENAME))
}

pub fn load_global_config() -> Result<GlobalConfig> {
    load_global_config_from_path(&global_config_path()?)
}

fn global_config_missing_message(path: &Path) -> String {
    format!(
        "global config '{}' does not exist; run `cpkg config init` first",
        path.display()
    )
}

pub(crate) fn load_global_config_from_path(path: &Path) -> Result<GlobalConfig> {
    if !path.exists() {
        return Ok(GlobalConfig::default());
    }

    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read global cpkg config '{}'", path.display()))?;
    let config = toml::from_str::<GlobalConfig>(&content)
        .with_context(|| format!("failed to parse global cpkg config '{}'", path.display()))?;
    validate_global_config(&config)?;
    Ok(config)
}

fn load_existing_global_config_from_path(path: &Path) -> Result<GlobalConfig> {
    if !path.exists() {
        anyhow::bail!("{}", global_config_missing_message(path));
    }
    load_global_config_from_path(path)
}

pub fn save_global_config(config: &GlobalConfig) -> Result<()> {
    save_global_config_to_path(&global_config_path()?, config)
}

pub(crate) fn save_global_config_to_path(path: &Path, config: &GlobalConfig) -> Result<()> {
    validate_global_config(config)?;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("invalid global cpkg config path"))?;
    fs::create_dir_all(parent).with_context(|| {
        format!(
            "failed to create global config directory '{}'",
            parent.display()
        )
    })?;
    let rendered = render_global_config(config)?;
    fs::write(path, rendered)
        .with_context(|| format!("failed to write global cpkg config '{}'", path.display()))?;
    Ok(())
}

pub fn show_global_config() -> Result<()> {
    let path = global_config_path()?;
    let term = Term::stdout();

    if !path.exists() {
        term.write_line(&format!("Global config: {}", path.display()))?;
        term.write_line("(not created yet; built-in defaults are active)")?;
        term.write_line("Run `cpkg config init` before editing global config.")?;
        return Ok(());
    }

    let config = load_global_config_from_path(&path)?;
    term.write_line(&format!("Global config: {}", path.display()))?;
    term.write_line(&render_global_config(&config)?.trim_end().to_string())?;
    Ok(())
}

pub fn init_global_config(force: bool) -> Result<()> {
    let path = global_config_path()?;
    if path.exists() && !force {
        anyhow::bail!(
            "global config '{}' already exists (use `cpkg config init --force` to overwrite)",
            path.display()
        );
    }

    let config = initialized_global_config_template();
    save_global_config_to_path(&path, &config)?;
    let term = Term::stdout();
    term.write_line(&format!("Initialized global config at {}", path.display()))?;
    Ok(())
}

fn describe_index_source(source: &IndexSourceConfig) -> String {
    match (source.path.as_deref(), source.url.as_deref()) {
        (Some(path), None) => format!("path={path}"),
        (None, Some(url)) => match source.cache_path.as_deref() {
            Some(cache_path) => format!("url={url}, cache_path={cache_path}"),
            None => format!("url={url}"),
        },
        (Some(path), Some(url)) => format!("path={path}, url={url}"),
        (None, None) => "(invalid source)".to_string(),
    }
}

fn validate_index_position(position: usize, len: usize, action: &str) -> Result<usize> {
    if position == 0 || position > len {
        anyhow::bail!(
            "cannot {action} global index source at position {}; valid positions are 1..={len}",
            position
        );
    }
    Ok(position - 1)
}

fn validate_insert_position(position: Option<usize>, len: usize) -> Result<usize> {
    match position {
        Some(position) if position == 0 || position > len + 1 => anyhow::bail!(
            "cannot insert global index source at position {}; valid positions are 1..={}",
            position,
            len + 1
        ),
        Some(position) => Ok(position - 1),
        None => Ok(len),
    }
}

pub fn show_global_index_sources() -> Result<()> {
    let path = global_config_path()?;
    let term = Term::stdout();

    term.write_line(&format!("Global config: {}", path.display()))?;
    if !path.exists() {
        term.write_line("(not created yet; built-in defaults are active)")?;
        term.write_line("1. built-in default index")?;
        term.write_line("Run `cpkg config init` before editing global config.")?;
        return Ok(());
    }

    let config = load_global_config_from_path(&path)?;
    if config.index.is_empty() {
        term.write_line(
            "Global index sources: (none configured; built-in default remains active)",
        )?;
        return Ok(());
    }

    term.write_line("Global index sources:")?;
    for (index, source) in config.index.iter().enumerate() {
        term.write_line(&format!("{}. {}", index + 1, describe_index_source(source)))?;
    }
    Ok(())
}

pub fn add_global_index_source(source: IndexSourceConfig, position: Option<usize>) -> Result<()> {
    let path = global_config_path()?;
    let inserted_at = add_index_source_to_config_path(&path, source.clone(), position)?;
    let term = Term::stdout();
    term.write_line(&format!(
        "Added global index source at position {} in {}",
        inserted_at,
        path.display()
    ))?;
    term.write_line(&format!("Source: {}", describe_index_source(&source)))?;
    Ok(())
}

pub fn set_global_index_source(position: usize, source: IndexSourceConfig) -> Result<()> {
    let path = global_config_path()?;
    set_index_source_in_config_path(&path, position, source.clone())?;
    let term = Term::stdout();
    term.write_line(&format!(
        "Updated global index source {} in {}",
        position,
        path.display()
    ))?;
    term.write_line(&format!("Source: {}", describe_index_source(&source)))?;
    Ok(())
}

pub fn remove_global_index_source(position: usize) -> Result<()> {
    let path = global_config_path()?;
    let removed = remove_index_source_from_config_path(&path, position)?;
    let term = Term::stdout();
    term.write_line(&format!(
        "Removed global index source {} from {}",
        position,
        path.display()
    ))?;
    term.write_line(&format!("Source: {}", describe_index_source(&removed)))?;
    Ok(())
}

pub fn move_global_index_source(from: usize, to: usize) -> Result<()> {
    let path = global_config_path()?;
    move_index_source_in_config_path(&path, from, to)?;
    let term = Term::stdout();
    term.write_line(&format!(
        "Moved global index source from position {} to {} in {}",
        from,
        to,
        path.display()
    ))?;
    Ok(())
}

pub fn set_global_org_source(
    name: &str,
    ssh_base: Option<String>,
    https_base: Option<String>,
    default_protocol: Option<SubmoduleProtocol>,
) -> Result<()> {
    let path = global_config_path()?;
    let updated =
        upsert_org_source_in_config_path(&path, name, ssh_base, https_base, default_protocol)?;
    let term = Term::stdout();
    term.write_line(&format!(
        "Updated global org source '{}' in {}",
        updated.name,
        path.display()
    ))?;
    Ok(())
}

pub fn remove_global_org_source(name: &str) -> Result<()> {
    let path = global_config_path()?;
    remove_org_source_from_config_path(&path, name)?;
    let term = Term::stdout();
    term.write_line(&format!(
        "Removed global org source '{}' from {}",
        name,
        path.display()
    ))?;
    Ok(())
}

pub fn set_global_default_org_source(name: &str) -> Result<()> {
    let path = global_config_path()?;
    set_default_org_source_in_config_path(&path, Some(name.to_string()))?;
    let term = Term::stdout();
    term.write_line(&format!(
        "Set global default org source to '{}' in {}",
        name,
        path.display()
    ))?;
    Ok(())
}

pub fn clear_global_default_org_source() -> Result<()> {
    let path = global_config_path()?;
    set_default_org_source_in_config_path(&path, None)?;
    let term = Term::stdout();
    term.write_line(&format!(
        "Cleared global default org source in {}",
        path.display()
    ))?;
    Ok(())
}

pub(crate) fn upsert_org_source_in_config_path(
    path: &Path,
    name: &str,
    ssh_base: Option<String>,
    https_base: Option<String>,
    default_protocol: Option<SubmoduleProtocol>,
) -> Result<NamedOrgSource> {
    let mut config = load_existing_global_config_from_path(path)?;

    let updated = match config.org.iter_mut().find(|source| source.name == name) {
        Some(source) => {
            if let Some(ssh_base) = ssh_base {
                source.ssh_base = Some(ssh_base);
            }
            if let Some(https_base) = https_base {
                source.https_base = Some(https_base);
            }
            if let Some(default_protocol) = default_protocol {
                source.default_protocol = default_protocol;
            }
            source.clone()
        }
        None => {
            let source = NamedOrgSource {
                name: name.to_string(),
                ssh_base,
                https_base,
                default_protocol: default_protocol.unwrap_or_default(),
            };
            config.org.push(source.clone());
            source
        }
    };

    validate_org_source(&updated, &format!("global [org.{}]", updated.name))?;
    config.org.sort_by(|left, right| left.name.cmp(&right.name));
    save_global_config_to_path(path, &config)?;
    Ok(updated)
}

pub(crate) fn add_index_source_to_config_path(
    path: &Path,
    source: IndexSourceConfig,
    position: Option<usize>,
) -> Result<usize> {
    validate_index_source(&source, "global [index] entry")?;
    let mut config = load_existing_global_config_from_path(path)?;
    let insert_at = validate_insert_position(position, config.index.len())?;
    config.index.insert(insert_at, source);
    save_global_config_to_path(path, &config)?;
    Ok(insert_at + 1)
}

pub(crate) fn set_index_source_in_config_path(
    path: &Path,
    position: usize,
    source: IndexSourceConfig,
) -> Result<()> {
    validate_index_source(&source, "global [index] entry")?;
    let mut config = load_existing_global_config_from_path(path)?;
    let index = validate_index_position(position, config.index.len(), "update")?;
    config.index[index] = source;
    save_global_config_to_path(path, &config)
}

pub(crate) fn remove_index_source_from_config_path(
    path: &Path,
    position: usize,
) -> Result<IndexSourceConfig> {
    let mut config = load_existing_global_config_from_path(path)?;
    let index = validate_index_position(position, config.index.len(), "remove")?;
    let removed = config.index.remove(index);
    save_global_config_to_path(path, &config)?;
    Ok(removed)
}

pub(crate) fn move_index_source_in_config_path(path: &Path, from: usize, to: usize) -> Result<()> {
    let mut config = load_existing_global_config_from_path(path)?;
    let from_index = validate_index_position(from, config.index.len(), "move")?;
    let to_index = validate_index_position(to, config.index.len(), "move")?;
    if from_index == to_index {
        return Ok(());
    }

    let source = config.index.remove(from_index);
    let insert_at = if from_index < to_index {
        to_index - 1
    } else {
        to_index
    };
    config.index.insert(insert_at, source);
    save_global_config_to_path(path, &config)
}

pub(crate) fn set_default_org_source_in_config_path(
    path: &Path,
    name: Option<String>,
) -> Result<()> {
    let mut config = load_existing_global_config_from_path(path)?;
    config.default_org = name;
    save_global_config_to_path(path, &config)
}

pub(crate) fn remove_org_source_from_config_path(path: &Path, name: &str) -> Result<()> {
    let mut config = load_existing_global_config_from_path(path)?;
    let original_len = config.org.len();
    config.org.retain(|source| source.name != name);
    if config.org.len() == original_len {
        anyhow::bail!("global org source '{}' not found", name);
    }
    if config.default_org.as_deref() == Some(name) {
        config.default_org = None;
    }
    save_global_config_to_path(path, &config)
}

#[cfg(test)]
mod tests {
    use super::{
        CURRENT_GLOBAL_CONFIG_FORMAT_VERSION, DEFAULT_GLOBAL_ORG_SOURCE_NAME, GlobalConfig,
        IndexSourceConfig, NamedOrgSource, add_index_source_to_config_path, home_dir_from_env,
        initialized_global_config_template, load_global_config_from_path,
        move_index_source_in_config_path, remove_index_source_from_config_path,
        remove_org_source_from_config_path, save_global_config_to_path,
        set_default_org_source_in_config_path, set_index_source_in_config_path,
        upsert_org_source_in_config_path, validate_global_config,
    };
    use crate::project::SubmoduleProtocol;
    use crate::project::index::{DEFAULT_INDEX_FILENAME, DEFAULT_INDEX_URL};
    use std::ffi::OsString;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_temp_dir(prefix: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "cpkg-config-{prefix}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn home_dir_prefers_home_then_userprofile_then_home_drive_pair() {
        assert_eq!(
            home_dir_from_env(
                Some(OsString::from("/home/test")),
                Some(OsString::from("C:\\Users\\test")),
                Some(OsString::from("C:")),
                Some(OsString::from("\\Users\\fallback")),
            )
            .unwrap(),
            PathBuf::from("/home/test")
        );

        assert_eq!(
            home_dir_from_env(
                None,
                Some(OsString::from("C:\\Users\\test")),
                Some(OsString::from("D:")),
                Some(OsString::from("\\Users\\fallback")),
            )
            .unwrap(),
            PathBuf::from("C:\\Users\\test")
        );

        assert_eq!(
            home_dir_from_env(
                None,
                None,
                Some(OsString::from("D:")),
                Some(OsString::from("\\Users\\fallback")),
            )
            .unwrap(),
            PathBuf::from("D:\\Users\\fallback")
        );
    }

    #[test]
    fn missing_config_path_loads_default_config() {
        let dir = make_temp_dir("load-default");
        let config = load_global_config_from_path(&dir.join("missing.toml")).unwrap();
        assert_eq!(config, GlobalConfig::default());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn validate_rejects_duplicate_org_names() {
        let error = validate_global_config(&GlobalConfig {
            format_version: CURRENT_GLOBAL_CONFIG_FORMAT_VERSION,
            default_org: None,
            index: Vec::new(),
            org: vec![
                NamedOrgSource {
                    name: "mirror".to_string(),
                    ssh_base: Some("git@example.com:mirror".to_string()),
                    https_base: Some("https://example.com/mirror".to_string()),
                    default_protocol: SubmoduleProtocol::Ssh,
                },
                NamedOrgSource {
                    name: "mirror".to_string(),
                    ssh_base: Some("git@example.com:mirror2".to_string()),
                    https_base: Some("https://example.com/mirror2".to_string()),
                    default_protocol: SubmoduleProtocol::Ssh,
                },
            ],
        })
        .unwrap_err();

        assert!(error.to_string().contains("duplicate global org source"));
    }

    #[test]
    fn validate_rejects_index_entry_without_location() {
        let error = validate_global_config(&GlobalConfig {
            format_version: CURRENT_GLOBAL_CONFIG_FORMAT_VERSION,
            default_org: None,
            index: vec![IndexSourceConfig::default()],
            org: Vec::new(),
        })
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("must set either `path` or `url`")
        );
    }

    #[test]
    fn upsert_org_source_updates_named_entry() {
        let dir = make_temp_dir("upsert-org");
        let path = dir.join("config.toml");
        save_global_config_to_path(
            &path,
            &GlobalConfig {
                format_version: CURRENT_GLOBAL_CONFIG_FORMAT_VERSION,
                default_org: None,
                index: Vec::new(),
                org: vec![NamedOrgSource {
                    name: "mirror".to_string(),
                    ssh_base: Some("git@example.com:mirror".to_string()),
                    https_base: Some("https://example.com/mirror".to_string()),
                    default_protocol: SubmoduleProtocol::Ssh,
                }],
            },
        )
        .unwrap();

        upsert_org_source_in_config_path(
            &path,
            "mirror",
            None,
            Some("https://example.com/new-mirror".to_string()),
            Some(SubmoduleProtocol::Https),
        )
        .unwrap();

        let config = load_global_config_from_path(&path).unwrap();
        assert_eq!(config.org.len(), 1);
        assert_eq!(config.org[0].default_protocol, SubmoduleProtocol::Https);
        assert_eq!(
            config.org[0].https_base.as_deref(),
            Some("https://example.com/new-mirror")
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn add_index_source_inserts_at_requested_position() {
        let dir = make_temp_dir("add-index");
        let path = dir.join("config.toml");
        save_global_config_to_path(
            &path,
            &GlobalConfig {
                format_version: CURRENT_GLOBAL_CONFIG_FORMAT_VERSION,
                default_org: None,
                index: vec![IndexSourceConfig {
                    path: Some("first.json".to_string()),
                    ..IndexSourceConfig::default()
                }],
                org: Vec::new(),
            },
        )
        .unwrap();

        let inserted_at = add_index_source_to_config_path(
            &path,
            IndexSourceConfig {
                path: Some("second.json".to_string()),
                ..IndexSourceConfig::default()
            },
            Some(1),
        )
        .unwrap();

        let config = load_global_config_from_path(&path).unwrap();
        assert_eq!(inserted_at, 1);
        assert_eq!(config.index.len(), 2);
        assert_eq!(config.index[0].path.as_deref(), Some("second.json"));
        assert_eq!(config.index[1].path.as_deref(), Some("first.json"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn set_index_source_replaces_existing_entry() {
        let dir = make_temp_dir("set-index");
        let path = dir.join("config.toml");
        save_global_config_to_path(
            &path,
            &GlobalConfig {
                format_version: CURRENT_GLOBAL_CONFIG_FORMAT_VERSION,
                default_org: None,
                index: vec![IndexSourceConfig {
                    path: Some("first.json".to_string()),
                    ..IndexSourceConfig::default()
                }],
                org: Vec::new(),
            },
        )
        .unwrap();

        set_index_source_in_config_path(
            &path,
            1,
            IndexSourceConfig {
                url: Some("https://example.com/index.json".to_string()),
                cache_path: Some("cache/index.json".to_string()),
                ..IndexSourceConfig::default()
            },
        )
        .unwrap();

        let config = load_global_config_from_path(&path).unwrap();
        assert_eq!(
            config.index[0].url.as_deref(),
            Some("https://example.com/index.json")
        );
        assert_eq!(
            config.index[0].cache_path.as_deref(),
            Some("cache/index.json")
        );
        assert!(config.index[0].path.is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn remove_index_source_returns_removed_entry() {
        let dir = make_temp_dir("remove-index");
        let path = dir.join("config.toml");
        save_global_config_to_path(
            &path,
            &GlobalConfig {
                format_version: CURRENT_GLOBAL_CONFIG_FORMAT_VERSION,
                default_org: None,
                index: vec![
                    IndexSourceConfig {
                        path: Some("first.json".to_string()),
                        ..IndexSourceConfig::default()
                    },
                    IndexSourceConfig {
                        path: Some("second.json".to_string()),
                        ..IndexSourceConfig::default()
                    },
                ],
                org: Vec::new(),
            },
        )
        .unwrap();

        let removed = remove_index_source_from_config_path(&path, 1).unwrap();
        let config = load_global_config_from_path(&path).unwrap();

        assert_eq!(removed.path.as_deref(), Some("first.json"));
        assert_eq!(config.index.len(), 1);
        assert_eq!(config.index[0].path.as_deref(), Some("second.json"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn move_index_source_reorders_entries() {
        let dir = make_temp_dir("move-index");
        let path = dir.join("config.toml");
        save_global_config_to_path(
            &path,
            &GlobalConfig {
                format_version: CURRENT_GLOBAL_CONFIG_FORMAT_VERSION,
                default_org: None,
                index: vec![
                    IndexSourceConfig {
                        path: Some("first.json".to_string()),
                        ..IndexSourceConfig::default()
                    },
                    IndexSourceConfig {
                        path: Some("second.json".to_string()),
                        ..IndexSourceConfig::default()
                    },
                    IndexSourceConfig {
                        path: Some("third.json".to_string()),
                        ..IndexSourceConfig::default()
                    },
                ],
                org: Vec::new(),
            },
        )
        .unwrap();

        move_index_source_in_config_path(&path, 3, 1).unwrap();

        let config = load_global_config_from_path(&path).unwrap();
        assert_eq!(config.index[0].path.as_deref(), Some("third.json"));
        assert_eq!(config.index[1].path.as_deref(), Some("first.json"));
        assert_eq!(config.index[2].path.as_deref(), Some("second.json"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn remove_org_source_deletes_named_entry() {
        let dir = make_temp_dir("remove-org");
        let path = dir.join("config.toml");
        save_global_config_to_path(
            &path,
            &GlobalConfig {
                format_version: CURRENT_GLOBAL_CONFIG_FORMAT_VERSION,
                default_org: Some("mirror".to_string()),
                index: Vec::new(),
                org: vec![NamedOrgSource {
                    name: "mirror".to_string(),
                    ssh_base: Some("git@example.com:mirror".to_string()),
                    https_base: Some("https://example.com/mirror".to_string()),
                    default_protocol: SubmoduleProtocol::Ssh,
                }],
            },
        )
        .unwrap();

        remove_org_source_from_config_path(&path, "mirror").unwrap();

        let config = load_global_config_from_path(&path).unwrap();
        assert!(config.org.is_empty());
        assert!(config.default_org.is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn validate_rejects_unknown_default_org() {
        let error = validate_global_config(&GlobalConfig {
            format_version: CURRENT_GLOBAL_CONFIG_FORMAT_VERSION,
            default_org: Some("missing".to_string()),
            index: Vec::new(),
            org: Vec::new(),
        })
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("global default org source 'missing'")
        );
    }

    #[test]
    fn initialized_template_contains_builtin_default_org() {
        let config = initialized_global_config_template();

        assert_eq!(
            config.default_org.as_deref(),
            Some(DEFAULT_GLOBAL_ORG_SOURCE_NAME)
        );
        assert_eq!(config.index.len(), 1);
        assert_eq!(config.index[0].url.as_deref(), Some(DEFAULT_INDEX_URL));
        assert_eq!(
            config.index[0].cache_path.as_deref(),
            Some(DEFAULT_INDEX_FILENAME)
        );
        assert_eq!(config.org.len(), 1);
        assert_eq!(config.org[0].name, DEFAULT_GLOBAL_ORG_SOURCE_NAME);
    }

    #[test]
    fn mutating_missing_config_requires_init_first() {
        let dir = make_temp_dir("missing-config-update");
        let path = dir.join("config.toml");

        let error = add_index_source_to_config_path(
            &path,
            IndexSourceConfig {
                path: Some("first.json".to_string()),
                ..IndexSourceConfig::default()
            },
            None,
        )
        .unwrap_err();

        assert!(error.to_string().contains("cpkg config init"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn set_default_org_source_updates_existing_config() {
        let dir = make_temp_dir("set-default-org");
        let path = dir.join("config.toml");
        save_global_config_to_path(
            &path,
            &GlobalConfig {
                format_version: CURRENT_GLOBAL_CONFIG_FORMAT_VERSION,
                default_org: None,
                index: Vec::new(),
                org: vec![
                    NamedOrgSource {
                        name: "mirror-a".to_string(),
                        ssh_base: Some("git@example.com:a".to_string()),
                        https_base: Some("https://example.com/a".to_string()),
                        default_protocol: SubmoduleProtocol::Ssh,
                    },
                    NamedOrgSource {
                        name: "mirror-b".to_string(),
                        ssh_base: Some("git@example.com:b".to_string()),
                        https_base: Some("https://example.com/b".to_string()),
                        default_protocol: SubmoduleProtocol::Https,
                    },
                ],
            },
        )
        .unwrap();

        set_default_org_source_in_config_path(&path, Some("mirror-b".to_string())).unwrap();

        let config = load_global_config_from_path(&path).unwrap();
        assert_eq!(config.default_org.as_deref(), Some("mirror-b"));
        let _ = fs::remove_dir_all(dir);
    }
}
