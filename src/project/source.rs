use anyhow::{Context, Result};

use crate::config::GlobalConfig;

use super::WtrProject;
use super::manifest::OrgSection;
use super::resolver::{RepositoryRemoteBases, SubmoduleProtocol};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedOrgSource {
    pub(crate) repository_bases: RepositoryRemoteBases,
    pub(crate) default_protocol: SubmoduleProtocol,
}

fn resolve_named_org_source(
    section: &OrgSection,
    global_config: &GlobalConfig,
    name: &str,
) -> Result<ResolvedOrgSource> {
    if section.ssh_base.is_some() || section.https_base.is_some() {
        anyhow::bail!("project [org] cannot set both `name` and custom org bases");
    }

    let source = global_config
        .org
        .iter()
        .find(|source| source.name == name)
        .with_context(|| format!("project [org] refers to unknown global org source '{name}'"))?;

    let resolved = ResolvedOrgSource {
        repository_bases: RepositoryRemoteBases {
            https_base: source.https_base.clone(),
            ssh_base: source.ssh_base.clone(),
        },
        default_protocol: section.protocol.unwrap_or(source.default_protocol),
    };
    resolved
        .repository_bases
        .ensure_protocol_supported(resolved.default_protocol)?;
    Ok(resolved)
}

fn resolve_project_org_source(section: &OrgSection) -> Result<ResolvedOrgSource> {
    let resolved = ResolvedOrgSource {
        repository_bases: RepositoryRemoteBases {
            https_base: section.https_base.clone(),
            ssh_base: section.ssh_base.clone(),
        },
        default_protocol: section.protocol.unwrap_or_default(),
    };
    resolved
        .repository_bases
        .ensure_protocol_supported(resolved.default_protocol)?;
    Ok(resolved)
}

fn resolve_builtin_org_source(section: &OrgSection) -> Result<ResolvedOrgSource> {
    let resolved = ResolvedOrgSource {
        repository_bases: RepositoryRemoteBases::wtr_default(),
        default_protocol: section.protocol.unwrap_or_default(),
    };
    resolved
        .repository_bases
        .ensure_protocol_supported(resolved.default_protocol)?;
    Ok(resolved)
}

pub(crate) fn resolve_org_source(
    manifest: &WtrProject,
    global_config: &GlobalConfig,
) -> Result<ResolvedOrgSource> {
    if let Some(name) = manifest.org.name.as_deref() {
        return resolve_named_org_source(&manifest.org, global_config, name);
    }

    if manifest.org.ssh_base.is_some() || manifest.org.https_base.is_some() {
        return resolve_project_org_source(&manifest.org);
    }

    if let Some(name) = global_config.default_org.as_deref() {
        return resolve_named_org_source(&manifest.org, global_config, name);
    }

    resolve_builtin_org_source(&manifest.org)
}

pub(crate) fn effective_protocol(
    source: &ResolvedOrgSource,
    protocol_override: Option<SubmoduleProtocol>,
) -> Result<SubmoduleProtocol> {
    let protocol = protocol_override.unwrap_or(source.default_protocol);
    source
        .repository_bases
        .ensure_protocol_supported(protocol)?;
    Ok(protocol)
}

#[cfg(test)]
mod tests {
    use super::{effective_protocol, resolve_org_source};
    use crate::config::{GlobalConfig, NamedOrgSource};
    use crate::project::{
        DependencySection, IndexSection, OrgSection, ProjectSection, SubmoduleProtocol, WtrProject,
    };

    fn manifest_with_org(org: OrgSection) -> WtrProject {
        WtrProject {
            format_version: 1,
            project: ProjectSection {
                name: "robot".to_string(),
                ioc_file: "robot.ioc".to_string(),
            },
            dependencies: DependencySection::default(),
            index: IndexSection::default(),
            org,
        }
    }

    #[test]
    fn resolve_org_source_uses_named_global_source() {
        let manifest = manifest_with_org(OrgSection {
            name: Some("mirror".to_string()),
            protocol: Some(SubmoduleProtocol::Https),
            ..OrgSection::default()
        });
        let global = GlobalConfig {
            default_org: None,
            org: vec![NamedOrgSource {
                name: "mirror".to_string(),
                ssh_base: Some("git@example.com:mirror".to_string()),
                https_base: Some("https://example.com/mirror".to_string()),
                default_protocol: SubmoduleProtocol::Ssh,
            }],
            ..GlobalConfig::default()
        };

        let resolved = resolve_org_source(&manifest, &global).unwrap();

        assert_eq!(resolved.default_protocol, SubmoduleProtocol::Https);
        assert_eq!(
            resolved.repository_bases.https_base.as_deref(),
            Some("https://example.com/mirror")
        );
    }

    #[test]
    fn resolve_org_source_supports_project_local_custom_bases() {
        let manifest = manifest_with_org(OrgSection {
            ssh_base: Some("git@example.com:project".to_string()),
            https_base: Some("https://example.com/project".to_string()),
            protocol: Some(SubmoduleProtocol::Https),
            ..OrgSection::default()
        });

        let resolved = resolve_org_source(&manifest, &GlobalConfig::default()).unwrap();

        assert_eq!(resolved.default_protocol, SubmoduleProtocol::Https);
        assert_eq!(
            resolved.repository_bases.ssh_base.as_deref(),
            Some("git@example.com:project")
        );
    }

    #[test]
    fn project_org_protocol_can_override_builtin_default_org() {
        let manifest = manifest_with_org(OrgSection {
            protocol: Some(SubmoduleProtocol::Https),
            ..OrgSection::default()
        });

        let resolved = resolve_org_source(&manifest, &GlobalConfig::default()).unwrap();
        let protocol = effective_protocol(&resolved, None).unwrap();

        assert_eq!(protocol, SubmoduleProtocol::Https);
        assert!(resolved.repository_bases.https_base.is_some());
    }

    #[test]
    fn resolve_org_source_uses_global_default_org_when_project_is_unspecified() {
        let manifest = manifest_with_org(OrgSection::default());
        let global = GlobalConfig {
            default_org: Some("mirror".to_string()),
            org: vec![NamedOrgSource {
                name: "mirror".to_string(),
                ssh_base: Some("git@example.com:mirror".to_string()),
                https_base: Some("https://example.com/mirror".to_string()),
                default_protocol: SubmoduleProtocol::Https,
            }],
            ..GlobalConfig::default()
        };

        let resolved = resolve_org_source(&manifest, &global).unwrap();

        assert_eq!(resolved.default_protocol, SubmoduleProtocol::Https);
        assert_eq!(
            resolved.repository_bases.ssh_base.as_deref(),
            Some("git@example.com:mirror")
        );
    }

    #[test]
    fn project_org_protocol_overrides_global_default_org_protocol() {
        let manifest = manifest_with_org(OrgSection {
            protocol: Some(SubmoduleProtocol::Ssh),
            ..OrgSection::default()
        });
        let global = GlobalConfig {
            default_org: Some("mirror".to_string()),
            org: vec![NamedOrgSource {
                name: "mirror".to_string(),
                ssh_base: Some("git@example.com:mirror".to_string()),
                https_base: Some("https://example.com/mirror".to_string()),
                default_protocol: SubmoduleProtocol::Https,
            }],
            ..GlobalConfig::default()
        };

        let resolved = resolve_org_source(&manifest, &global).unwrap();

        assert_eq!(resolved.default_protocol, SubmoduleProtocol::Ssh);
        assert_eq!(
            resolved.repository_bases.https_base.as_deref(),
            Some("https://example.com/mirror")
        );
    }

    #[test]
    fn cli_protocol_override_wins_over_source_default() {
        let manifest = manifest_with_org(OrgSection {
            protocol: Some(SubmoduleProtocol::Https),
            ..OrgSection::default()
        });
        let resolved = resolve_org_source(&manifest, &GlobalConfig::default()).unwrap();

        let protocol = effective_protocol(&resolved, Some(SubmoduleProtocol::Ssh)).unwrap();

        assert_eq!(protocol, SubmoduleProtocol::Ssh);
    }
}
