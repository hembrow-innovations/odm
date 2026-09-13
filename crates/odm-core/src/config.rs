use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::OdmError;
use crate::io::atomic_write;
use crate::paths::{parse_path_token, resolve_under_root, PathResolveError};

pub use crate::paths::{config_path, odm_dir, pin_path};

/// Canonical Workspace config (`.odm/odm.config.yaml`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// When omitted, defaults to true via [`WorkspaceConfig::manage_gitignore`].
    #[serde(
        default,
        skip_serializing_if = "skip_manage_gitignore",
        rename = "manage_gitignore"
    )]
    pub manage_gitignore: Option<bool>,

    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub projects: BTreeMap<String, ProjectEntry>,

    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub progens: BTreeMap<String, ProgenEntry>,

    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub progen_groups: BTreeMap<String, Vec<String>>,

    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub actions: BTreeMap<String, String>,

    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub generators: BTreeMap<String, String>,
}

fn skip_manage_gitignore(v: &Option<bool>) -> bool {
    match v {
        None | Some(true) => true,
        Some(false) => false,
    }
}

impl WorkspaceConfig {
    pub fn manage_gitignore(&self) -> bool {
        self.manage_gitignore.unwrap_or(true)
    }

    pub fn minimal() -> Self {
        Self::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckoutMode {
    #[default]
    Clone,
    Gitlink,
}

impl CheckoutMode {
    fn is_clone(&self) -> bool {
        matches!(self, Self::Clone)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectEntry {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "type")]
    pub type_: Option<String>,
    #[serde(default, skip_serializing_if = "CheckoutMode::is_clone")]
    pub checkout: CheckoutMode,
}

impl Default for ProjectEntry {
    fn default() -> Self {
        Self {
            path: String::new(),
            url: None,
            branch: None,
            type_: None,
            checkout: CheckoutMode::Clone,
        }
    }
}

impl ProjectEntry {
    pub fn is_managed(&self) -> bool {
        self.url.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgenEntry {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "CheckoutMode::is_clone")]
    pub checkout: CheckoutMode,
}

impl Default for ProgenEntry {
    fn default() -> Self {
        Self {
            path: String::new(),
            url: None,
            branch: None,
            checkout: CheckoutMode::Clone,
        }
    }
}

impl ProgenEntry {
    pub fn is_managed(&self) -> bool {
        self.url.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionTask {
    pub run: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionDef {
    pub tasks: Vec<ActionTask>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeneratorDef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Loaded Workspace: root path + validated config + merged bundles.
#[derive(Debug, Clone)]
pub struct Workspace {
    pub root: PathBuf,
    pub config: WorkspaceConfig,
    pub actions: BTreeMap<String, ActionDef>,
    pub generators: BTreeMap<String, GeneratorDef>,
}

impl Workspace {
    pub fn config_path(&self) -> PathBuf {
        config_path(&self.root)
    }

    pub fn pin_path(&self) -> PathBuf {
        pin_path(&self.root)
    }
}

/// Deserialize + validate config at `root` (eager bundle load).
pub fn load_workspace(root: &Path) -> Result<Workspace, OdmError> {
    let path = config_path(root);
    if !path.is_file() {
        return Err(OdmError::workspace(format!(
            "not a Workspace: missing {}",
            path.display()
        )));
    }
    let text = fs::read_to_string(&path)
        .map_err(|e| OdmError::workspace(format!("failed to read {}: {e}", path.display())))?;
    let config = parse_config_yaml(&text)?;
    validate_and_load_bundles(root, config)
}

pub fn parse_config_yaml(text: &str) -> Result<WorkspaceConfig, OdmError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(WorkspaceConfig::default());
    }
    serde_yaml::from_str(trimmed)
        .map_err(|e| OdmError::workspace(format!("invalid config YAML: {e}")))
}

pub fn validate_and_load_bundles(
    root: &Path,
    config: WorkspaceConfig,
) -> Result<Workspace, OdmError> {
    validate_entity_names_and_paths(&config)?;
    validate_checkout(&config)?;
    validate_progen_groups(&config)?;
    let actions = load_action_bundles(root, &config.actions)?;
    let generators = load_generator_bundles(root, &config.generators)?;
    Ok(Workspace {
        root: root.to_path_buf(),
        config,
        actions,
        generators,
    })
}

fn validate_entity_names_and_paths(config: &WorkspaceConfig) -> Result<(), OdmError> {
    for (name, entry) in &config.projects {
        require_entity_name("project", name)?;
        validate_rel_path("project", name, &entry.path)?;
    }
    for (name, entry) in &config.progens {
        require_entity_name("progen", name)?;
        validate_rel_path("progen", name, &entry.path)?;
        if config.projects.contains_key(name) {
            return Err(OdmError::workspace(format!(
                "name '{name}' is used for both a project and a progen"
            )));
        }
    }
    for name in config.progen_groups.keys() {
        require_non_empty_name("progen_group", name)?;
    }
    for name in config.actions.keys() {
        require_non_empty_name("actions bundle", name)?;
    }
    for name in config.generators.keys() {
        require_non_empty_name("generators bundle", name)?;
    }
    Ok(())
}

fn require_non_empty_name(kind: &str, name: &str) -> Result<(), OdmError> {
    if name.trim().is_empty() {
        return Err(OdmError::workspace(format!(
            "{kind} name must not be empty"
        )));
    }
    Ok(())
}

/// Project/Progen map key: path-safe token (same rules as worktree slot names).
pub fn require_entity_name(kind: &str, name: &str) -> Result<(), OdmError> {
    match parse_path_token(name) {
        Ok(_) => Ok(()),
        Err("empty") => Err(OdmError::workspace(format!(
            "{kind} name must not be empty"
        ))),
        Err("dot") => Err(OdmError::workspace(format!(
            "invalid {kind} name '{}'",
            name.trim()
        ))),
        Err("separator") => Err(OdmError::workspace(format!(
            "invalid {kind} name '{}': path separators not allowed",
            name.trim()
        ))),
        Err(_) => Err(OdmError::workspace(format!(
            "invalid {kind} name '{}': must be a simple name",
            name.trim()
        ))),
    }
}

fn validate_rel_path(kind: &str, name: &str, path: &str) -> Result<(), OdmError> {
    if path.trim().is_empty() {
        return Err(OdmError::workspace(format!(
            "{kind} '{name}' path must not be empty"
        )));
    }
    // Same escape rules as runtime checkout resolve (`paths::resolve_under_root`).
    resolve_under_root(Path::new("/__odm_ws__"), path).map_err(|e| match e {
        PathResolveError::Absolute { path } => OdmError::workspace(format!(
            "{kind} '{name}' path must be relative, got '{path}'"
        )),
        PathResolveError::Escape { path } => OdmError::workspace(format!(
            "{kind} '{name}' path must not escape Workspace root, got '{path}'"
        )),
    })?;
    Ok(())
}

fn validate_action_task_dir(action: &str, task_i: usize, dir: &str) -> Result<(), OdmError> {
    if dir.trim().is_empty() {
        return Err(OdmError::workspace(format!(
            "action '{action}' task {task_i} dir must not be empty"
        )));
    }
    resolve_under_root(Path::new("/__odm_ws__"), dir).map_err(|e| match e {
        PathResolveError::Absolute { path } => OdmError::workspace(format!(
            "action '{action}' task {task_i} dir must be relative, got '{path}'"
        )),
        PathResolveError::Escape { path } => OdmError::workspace(format!(
            "action '{action}' task {task_i} dir must not escape Workspace root, got '{path}'"
        )),
    })?;
    Ok(())
}

fn url_blank(url: Option<&str>) -> bool {
    url.map(|u| u.trim().is_empty()).unwrap_or(true)
}

fn path_is_under(child: &str, parent: &str) -> bool {
    let child = Path::new(child);
    let parent = Path::new(parent);
    child.starts_with(parent) && child != parent
}

fn validate_checkout(config: &WorkspaceConfig) -> Result<(), OdmError> {
    let mut managed_paths: Vec<(&str, &str)> = Vec::new();
    for (name, e) in &config.projects {
        if e.is_managed() {
            managed_paths.push((name.as_str(), e.path.as_str()));
        }
    }
    for (name, e) in &config.progens {
        if e.is_managed() {
            managed_paths.push((name.as_str(), e.path.as_str()));
        }
    }
    for (name, e) in &config.projects {
        reject_bad_gitlink(
            "project",
            name,
            &e.path,
            e.url.as_deref(),
            e.checkout,
            &managed_paths,
        )?;
    }
    for (name, e) in &config.progens {
        reject_bad_gitlink(
            "progen",
            name,
            &e.path,
            e.url.as_deref(),
            e.checkout,
            &managed_paths,
        )?;
    }
    Ok(())
}

fn reject_bad_gitlink(
    kind: &str,
    name: &str,
    path: &str,
    url: Option<&str>,
    checkout: CheckoutMode,
    managed_paths: &[(&str, &str)],
) -> Result<(), OdmError> {
    if checkout != CheckoutMode::Gitlink {
        return Ok(());
    }
    if url_blank(url) {
        return Err(OdmError::workspace(format!(
            "{kind} '{name}' checkout gitlink requires url"
        )));
    }
    for (other_name, other_path) in managed_paths {
        if *other_name == name {
            continue;
        }
        if path_is_under(path, other_path) {
            return Err(OdmError::workspace(format!(
                "{kind} '{name}' gitlink path '{path}' is nested under managed path '{other_path}'"
            )));
        }
    }
    Ok(())
}

fn validate_progen_groups(config: &WorkspaceConfig) -> Result<(), OdmError> {
    for (group, members) in &config.progen_groups {
        for m in members {
            if !config.progens.contains_key(m) {
                return Err(OdmError::workspace(format!(
                    "progen_group '{group}' references unknown progen '{m}'"
                )));
            }
        }
    }
    Ok(())
}

fn resolve_bundle_path(
    root: &Path,
    kind: &str,
    bundle_name: &str,
    rel: &str,
) -> Result<PathBuf, OdmError> {
    resolve_under_root(root, rel).map_err(|e| match e {
        PathResolveError::Absolute { path } => OdmError::workspace(format!(
            "{kind} bundle '{bundle_name}' path must be relative, got '{path}'"
        )),
        PathResolveError::Escape { path } => OdmError::workspace(format!(
            "{kind} bundle '{bundle_name}' path must not escape Workspace root, got '{path}'"
        )),
    })
}

fn load_action_bundles(
    root: &Path,
    bundles: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, ActionDef>, OdmError> {
    let mut merged = BTreeMap::new();
    for (bundle_name, rel) in bundles {
        let path = resolve_bundle_path(root, "action", bundle_name, rel)?;
        if !path.is_file() {
            return Err(OdmError::workspace(format!(
                "action bundle '{bundle_name}' path does not exist: {rel}"
            )));
        }
        let text = fs::read_to_string(&path)
            .map_err(|e| OdmError::workspace(format!("failed to read action bundle {rel}: {e}")))?;
        let map: BTreeMap<String, ActionDef> = if text.trim().is_empty() {
            BTreeMap::new()
        } else {
            serde_yaml::from_str(&text)
                .map_err(|e| OdmError::workspace(format!("invalid action bundle {rel}: {e}")))?
        };
        for (name, def) in map {
            if merged.contains_key(&name) {
                return Err(OdmError::workspace(format!(
                    "duplicate action name '{name}' (also in bundle '{bundle_name}')"
                )));
            }
            if def.tasks.is_empty() {
                return Err(OdmError::workspace(format!(
                    "action '{name}' tasks must not be empty"
                )));
            }
            for (i, task) in def.tasks.iter().enumerate() {
                if task.run.trim().is_empty() {
                    return Err(OdmError::workspace(format!(
                        "action '{name}' task {i} run must not be empty"
                    )));
                }
                if let Some(ref dir) = task.dir {
                    validate_action_task_dir(&name, i, dir)?;
                }
            }
            merged.insert(name, def);
        }
    }
    Ok(merged)
}

fn load_generator_bundles(
    root: &Path,
    bundles: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, GeneratorDef>, OdmError> {
    let mut merged = BTreeMap::new();
    for (bundle_name, rel) in bundles {
        let path = resolve_bundle_path(root, "generator", bundle_name, rel)?;
        if !path.is_file() {
            return Err(OdmError::workspace(format!(
                "generator bundle '{bundle_name}' path does not exist: {rel}"
            )));
        }
        let text = fs::read_to_string(&path).map_err(|e| {
            OdmError::workspace(format!("failed to read generator bundle {rel}: {e}"))
        })?;
        let map: BTreeMap<String, GeneratorDef> = if text.trim().is_empty() {
            BTreeMap::new()
        } else {
            serde_yaml::from_str(&text)
                .map_err(|e| OdmError::workspace(format!("invalid generator bundle {rel}: {e}")))?
        };
        for (name, def) in map {
            if merged.contains_key(&name) {
                return Err(OdmError::workspace(format!(
                    "duplicate generator name '{name}' (also in bundle '{bundle_name}')"
                )));
            }
            if def
                .template
                .as_ref()
                .map(|s| s.trim().is_empty())
                .unwrap_or(true)
                && def
                    .url
                    .as_ref()
                    .map(|s| s.trim().is_empty())
                    .unwrap_or(true)
            {
                return Err(OdmError::workspace(format!(
                    "generator '{name}' needs template and/or url"
                )));
            }
            merged.insert(name, def);
        }
    }
    Ok(merged)
}

/// Serialize and atomically write Workspace config.
pub fn save_config(root: &Path, config: &WorkspaceConfig) -> Result<(), OdmError> {
    let path = config_path(root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            OdmError::operation(format!("failed to create {}: {e}", parent.display()))
        })?;
    }
    let yaml = serde_yaml::to_string(config)
        .map_err(|e| OdmError::operation(format!("failed to serialize config: {e}")))?;
    atomic_write(&path, &yaml)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn empty_and_minimal_valid() {
        let c = parse_config_yaml("").unwrap();
        assert!(c.projects.is_empty());
        let c = parse_config_yaml("{}").unwrap();
        assert!(c.manage_gitignore());
        let c = parse_config_yaml("name: demo\n").unwrap();
        assert_eq!(c.name.as_deref(), Some("demo"));
    }

    #[test]
    fn deny_unknown_fields() {
        let err = parse_config_yaml("foo: 1\n").unwrap_err();
        assert!(matches!(err, OdmError::Workspace(_)));
    }

    #[test]
    fn project_roundtrip_sorted() {
        let mut c = WorkspaceConfig::default();
        c.projects.insert(
            "beta".into(),
            ProjectEntry {
                path: "b".into(),
                url: None,
                branch: None,
                type_: None,
                checkout: CheckoutMode::Clone,
            },
        );
        c.projects.insert(
            "alpha".into(),
            ProjectEntry {
                path: "a".into(),
                url: Some("https://example.com/a.git".into()),
                branch: Some("main".into()),
                type_: Some("service".into()),
                checkout: CheckoutMode::Clone,
            },
        );
        let yaml = serde_yaml::to_string(&c).unwrap();
        assert!(yaml.find("alpha").unwrap() < yaml.find("beta").unwrap());
        assert!(!yaml.contains("manage_gitignore"));
        let back: WorkspaceConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn manage_gitignore_false_written() {
        let c = WorkspaceConfig {
            manage_gitignore: Some(false),
            ..Default::default()
        };
        let yaml = serde_yaml::to_string(&c).unwrap();
        assert!(yaml.contains("manage_gitignore: false"));
    }

    #[test]
    fn absolute_path_rejected() {
        let yaml = "projects:\n  x:\n    path: /abs\n";
        let c = parse_config_yaml(yaml).unwrap();
        let dir = tempdir().unwrap();
        let err = validate_and_load_bundles(dir.path(), c).unwrap_err();
        assert!(err.to_string().contains("relative"));
    }

    #[test]
    fn escape_path_rejected() {
        let yaml = "projects:\n  x:\n    path: ../outside\n";
        let c = parse_config_yaml(yaml).unwrap();
        let dir = tempdir().unwrap();
        let err = validate_and_load_bundles(dir.path(), c).unwrap_err();
        assert!(err.to_string().contains("escape"));
    }

    #[test]
    fn progen_group_unknown_member() {
        let yaml = r#"
progens:
  a:
    path: docs
progen_groups:
  g:
    - missing
"#;
        let c = parse_config_yaml(yaml).unwrap();
        let dir = tempdir().unwrap();
        let err = validate_and_load_bundles(dir.path(), c).unwrap_err();
        assert!(err.to_string().contains("unknown progen"));
    }

    #[test]
    fn action_bundle_missing_path() {
        let dir = tempdir().unwrap();
        let c = parse_config_yaml("actions:\n  core: actions/missing.yaml\n").unwrap();
        let err = validate_and_load_bundles(dir.path(), c).unwrap_err();
        assert!(err.to_string().contains("path does not exist"));
        assert!(matches!(err, OdmError::Workspace(_)));
        assert_eq!(crate::error::exit_code(&err), 2);
    }

    #[test]
    fn action_bundle_absolute_path_rejected() {
        let dir = tempdir().unwrap();
        let abs = dir.path().join("evil.yaml");
        fs::write(&abs, "boot:\n  tasks:\n    - run: echo hi\n").unwrap();
        let yaml = format!("actions:\n  core: {}\n", abs.display());
        let c = parse_config_yaml(&yaml).unwrap();
        let err = validate_and_load_bundles(dir.path(), c).unwrap_err();
        assert!(err.to_string().contains("relative"), "{err}");
        assert!(matches!(err, OdmError::Workspace(_)));
        assert_eq!(crate::error::exit_code(&err), 2);
    }

    #[test]
    fn action_bundle_escape_path_rejected() {
        let dir = tempdir().unwrap();
        let outside = dir.path().parent().unwrap().join("outside-actions.yaml");
        fs::write(&outside, "boot:\n  tasks:\n    - run: echo hi\n").unwrap();
        let c = parse_config_yaml("actions:\n  core: ../outside-actions.yaml\n").unwrap();
        let err = validate_and_load_bundles(dir.path(), c).unwrap_err();
        assert!(err.to_string().contains("escape"), "{err}");
        assert!(matches!(err, OdmError::Workspace(_)));
        assert_eq!(crate::error::exit_code(&err), 2);
        let _ = fs::remove_file(&outside);
    }

    #[test]
    fn generator_bundle_absolute_and_escape_rejected() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let abs = root.join("evil-g.yaml");
        fs::write(&abs, "pkg:\n  template: t.md\n").unwrap();
        let yaml = format!("generators:\n  core: {}\n", abs.display());
        let c = parse_config_yaml(&yaml).unwrap();
        let err = validate_and_load_bundles(root, c).unwrap_err();
        assert!(err.to_string().contains("relative"), "{err}");
        assert!(matches!(err, OdmError::Workspace(_)));
        assert_eq!(crate::error::exit_code(&err), 2);

        let outside = root.parent().unwrap().join("outside-gen.yaml");
        fs::write(&outside, "pkg:\n  template: t.md\n").unwrap();
        let c = parse_config_yaml("generators:\n  core: ../outside-gen.yaml\n").unwrap();
        let err = validate_and_load_bundles(root, c).unwrap_err();
        assert!(err.to_string().contains("escape"), "{err}");
        assert!(matches!(err, OdmError::Workspace(_)));
        assert_eq!(crate::error::exit_code(&err), 2);
        let _ = fs::remove_file(&outside);
    }

    #[test]
    fn action_bundle_eager_and_dedupe() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("actions")).unwrap();
        let mut f = fs::File::create(root.join("actions/core.yaml")).unwrap();
        writeln!(
            f,
            "boot:\n  tasks:\n    - run: echo hi\nchain:\n  tasks:\n    - run: step1\n    - run: step2\n      dir: sub\n"
        )
        .unwrap();
        let yaml = "actions:\n  core: actions/core.yaml\n";
        let c = parse_config_yaml(yaml).unwrap();
        let ws = validate_and_load_bundles(root, c).unwrap();
        assert_eq!(ws.actions["boot"].tasks.len(), 1);
        assert_eq!(ws.actions["boot"].tasks[0].run, "echo hi");
        assert_eq!(ws.actions["chain"].tasks.len(), 2);
        assert_eq!(ws.actions["chain"].tasks[1].dir.as_deref(), Some("sub"));

        let mut f2 = fs::File::create(root.join("actions/other.yaml")).unwrap();
        writeln!(f2, "boot:\n  tasks:\n    - run: other\n").unwrap();
        let yaml2 = "actions:\n  core: actions/core.yaml\n  other: actions/other.yaml\n";
        let c2 = parse_config_yaml(yaml2).unwrap();
        let err = validate_and_load_bundles(root, c2).unwrap_err();
        assert!(err.to_string().contains("duplicate action"));
    }

    #[test]
    fn action_empty_tasks_rejected() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("actions")).unwrap();
        fs::write(root.join("actions/core.yaml"), "empty:\n  tasks: []\n").unwrap();
        let c = parse_config_yaml("actions:\n  core: actions/core.yaml\n").unwrap();
        let err = validate_and_load_bundles(root, c).unwrap_err();
        assert!(err.to_string().contains("tasks must not be empty"));
    }

    #[test]
    fn action_empty_run_rejected() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("actions")).unwrap();
        fs::write(
            root.join("actions/core.yaml"),
            "bad:\n  tasks:\n    - run: \"  \"\n",
        )
        .unwrap();
        let c = parse_config_yaml("actions:\n  core: actions/core.yaml\n").unwrap();
        let err = validate_and_load_bundles(root, c).unwrap_err();
        assert!(err.to_string().contains("run must not be empty"));
    }

    #[test]
    fn action_task_dir_escape_rejected() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("actions")).unwrap();
        fs::write(
            root.join("actions/core.yaml"),
            "bad:\n  tasks:\n    - run: true\n      dir: ../outside\n",
        )
        .unwrap();
        let c = parse_config_yaml("actions:\n  core: actions/core.yaml\n").unwrap();
        let err = validate_and_load_bundles(root, c).unwrap_err();
        assert!(err.to_string().contains("escape"), "{err}");
        assert!(matches!(err, OdmError::Workspace(_)));
        assert_eq!(crate::error::exit_code(&err), 2);
    }

    #[test]
    fn action_task_dir_absolute_rejected() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("actions")).unwrap();
        fs::write(
            root.join("actions/core.yaml"),
            "bad:\n  tasks:\n    - run: true\n      dir: /tmp/abs\n",
        )
        .unwrap();
        let c = parse_config_yaml("actions:\n  core: actions/core.yaml\n").unwrap();
        let err = validate_and_load_bundles(root, c).unwrap_err();
        assert!(err.to_string().contains("relative"), "{err}");
        assert!(matches!(err, OdmError::Workspace(_)));
        assert_eq!(crate::error::exit_code(&err), 2);
    }

    #[test]
    fn action_task_dir_in_workspace_ok() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("actions")).unwrap();
        fs::write(
            root.join("actions/core.yaml"),
            "ok:\n  tasks:\n    - run: true\n      dir: projects/alpha\n",
        )
        .unwrap();
        let c = parse_config_yaml("actions:\n  core: actions/core.yaml\n").unwrap();
        let ws = validate_and_load_bundles(root, c).unwrap();
        assert_eq!(
            ws.actions["ok"].tasks[0].dir.as_deref(),
            Some("projects/alpha")
        );
    }

    #[test]
    fn generator_needs_source() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("g")).unwrap();
        fs::write(root.join("g/g.yaml"), "pkg: {}\n").unwrap();
        let c = parse_config_yaml("generators:\n  core: g/g.yaml\n").unwrap();
        let err = validate_and_load_bundles(root, c).unwrap_err();
        assert!(err.to_string().contains("template and/or url"));
    }

    #[test]
    fn save_load_atomic() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let c = WorkspaceConfig {
            name: Some("t".into()),
            ..Default::default()
        };
        save_config(root, &c).unwrap();
        let ws = load_workspace(root).unwrap();
        assert_eq!(ws.config.name.as_deref(), Some("t"));
    }

    #[test]
    fn type_field_rename() {
        let yaml = "projects:\n  p:\n    path: x\n    type: lib\n";
        let c = parse_config_yaml(yaml).unwrap();
        assert_eq!(c.projects["p"].type_.as_deref(), Some("lib"));
    }

    #[test]
    fn cross_map_duplicate_name_rejected() {
        let yaml = r#"
projects:
  shared:
    path: projects/shared
progens:
  shared:
    path: progens/shared
"#;
        let c = parse_config_yaml(yaml).unwrap();
        let dir = tempdir().unwrap();
        let err = validate_and_load_bundles(dir.path(), c).unwrap_err();
        assert!(
            err.to_string().contains("both a project and a progen"),
            "{err}"
        );
        assert!(matches!(err, OdmError::Workspace(_)));
    }

    #[test]
    fn entity_name_path_unsafe_rejected() {
        let dir = tempdir().unwrap();
        let bad_names: &[&str] = &["a/b", "..", ".", r"a\b", "a\0b"];
        for bad in bad_names {
            let mut c = WorkspaceConfig::default();
            c.projects.insert(
                (*bad).into(),
                ProjectEntry {
                    path: "p".into(),
                    url: None,
                    branch: None,
                    type_: None,
                    checkout: CheckoutMode::Clone,
                },
            );
            let err = validate_and_load_bundles(dir.path(), c).unwrap_err();
            assert!(
                err.to_string().contains("invalid project name"),
                "bad={bad:?} err={err}"
            );
        }
        let yaml = "projects:\n  good-name:\n    path: p\nprogens:\n  other:\n    path: q\n";
        let c = parse_config_yaml(yaml).unwrap();
        validate_and_load_bundles(dir.path(), c).unwrap();
    }

    #[test]
    fn checkout_mode() {
        let omitted = "projects:\n  p:\n    path: x\n    url: https://example.com/p.git\n";
        let c = parse_config_yaml(omitted).unwrap();
        assert_eq!(c.projects["p"].checkout, CheckoutMode::Clone);
        let yaml = serde_yaml::to_string(&c).unwrap();
        assert!(!yaml.contains("checkout"), "{yaml}");
        let back: WorkspaceConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(back.projects["p"].checkout, CheckoutMode::Clone);

        let gitlink = r#"
projects:
  p:
    path: vendor/p
    url: https://example.com/p.git
    checkout: gitlink
progens:
  g:
    path: vendor/g
    url: https://example.com/g.git
    checkout: gitlink
"#;
        let c = parse_config_yaml(gitlink).unwrap();
        assert_eq!(c.projects["p"].checkout, CheckoutMode::Gitlink);
        assert_eq!(c.progens["g"].checkout, CheckoutMode::Gitlink);
        let dir = tempdir().unwrap();
        validate_and_load_bundles(dir.path(), c.clone()).unwrap();
        let out = serde_yaml::to_string(&c).unwrap();
        assert!(out.contains("checkout: gitlink"), "{out}");

        let nested = r#"
projects:
  parent:
    path: vendor
    url: https://example.com/parent.git
  child:
    path: vendor/nested
    url: https://example.com/child.git
    checkout: gitlink
"#;
        let c = parse_config_yaml(nested).unwrap();
        let err = validate_and_load_bundles(dir.path(), c).unwrap_err();
        assert!(err.to_string().contains("nested"), "{err}");
        assert!(matches!(err, OdmError::Workspace(_)));
    }

    #[test]
    fn gitlink_requires_url() {
        let dir = tempdir().unwrap();
        let project = "projects:\n  p:\n    path: x\n    checkout: gitlink\n";
        let c = parse_config_yaml(project).unwrap();
        let err = validate_and_load_bundles(dir.path(), c).unwrap_err();
        assert!(err.to_string().contains("url"), "{err}");
        assert!(matches!(err, OdmError::Workspace(_)));

        let progen = "progens:\n  g:\n    path: docs\n    checkout: gitlink\n";
        let c = parse_config_yaml(progen).unwrap();
        let err = validate_and_load_bundles(dir.path(), c).unwrap_err();
        assert!(err.to_string().contains("url"), "{err}");
        assert!(matches!(err, OdmError::Workspace(_)));
    }
}
