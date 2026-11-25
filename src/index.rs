use crate::boot_metadata::{PropMeta, parse_metadata_json};
use crate::model::{
    ActiveStarters, BeanDef, BeanGraphEdge, EventKind, FileFacts, InjectionSite, ResolvedBeanTarget,
    SpringProjectModel, WiringStatus,
};
use crate::parser_config::{is_spring_config_path, parse_config_file};
use crate::parser_java::parse_jvm_source;
use crate::parser_xml::parse_spring_xml;
use crate::text_util::normalize_relaxed_key;
use openidea_plugin::indexing::{Alias, Anchor, Document as IndexDoc, Edges};
use openidea_plugin::{DependencyNode, Project, host};
use std::collections::{BTreeMap, BTreeSet, HashMap};

const MAX_SCAN_FILES_PER_GLOB: u32 = 800;
const MAX_TOTAL_SCAN_FILES: usize = 1500;
const MAX_SCAN_FILE_BYTES: usize = 512 * 1024;
const MAX_PARSE_BYTES: usize = 1024 * 1024;
const AUTO_TASK_THRESHOLD: usize = 25;

pub fn content_hash(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in text.as_bytes() {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[derive(Default)]
pub struct ProjectCache {
    pub projects: BTreeMap<String, SpringProjectModel>,
}

impl ProjectCache {
    pub fn get_or_init(&mut self, root: &str) -> &mut SpringProjectModel {
        self.projects
            .entry(root.to_owned())
            .or_insert_with(|| SpringProjectModel {
                root: root.to_owned(),
                ..SpringProjectModel::default()
            })
    }

    pub fn insert_file_facts(&mut self, root: &str, path: &str, hash: u64, facts: FileFacts) {
        let model = self.get_or_init(root);
        model.files.insert(path.to_owned(), facts);
        model.file_hashes.insert(path.to_owned(), hash);
        model.graph_dirty = true;
    }

    pub fn update_file_lazy(&mut self, root: &str, path: &str, language: &str, text: &str) {
        if !should_index_path(path) {
            return;
        }
        let hash = content_hash(text);
        if let Some(model) = self.projects.get(root)
            && model.file_hashes.get(path) == Some(&hash)
            && model.files.contains_key(path)
        {
            return;
        }
        let facts = parse_file(path, language, text);
        self.insert_file_facts(root, path, hash, facts);
    }

    pub fn remove_file_lazy(&mut self, root: &str, path: &str) {
        if let Some(model) = self.projects.get_mut(root) {
            model.files.remove(path);
            model.file_hashes.remove(path);
            model.graph_dirty = true;
        }
    }

    pub fn update_file(&mut self, root: &str, path: &str, language: &str, text: &str, active_profile: Option<&str>) {
        self.update_file_lazy(root, path, language, text);
        if let Some(model) = self.projects.get_mut(root) {
            model.ensure_bean_graph(active_profile);
        }
    }

    pub fn remove_file(&mut self, root: &str, path: &str, active_profile: Option<&str>) {
        self.remove_file_lazy(root, path);
        if let Some(model) = self.projects.get_mut(root) {
            model.ensure_bean_graph(active_profile);
        }
    }

    pub fn resolve_root_for_doc(&self, doc_path: &str) -> String {
        self.projects
            .keys()
            .filter(|r| doc_path.starts_with(r.as_str()))
            .max_by_key(|r| r.len())
            .cloned()
            .unwrap_or_else(|| {
                doc_path
                    .rsplit_once('/')
                    .map(|(dir, _)| dir.to_owned())
                    .unwrap_or_else(|| "/".to_owned())
            })
    }

    pub fn model_for_doc(&mut self, doc_path: &str, doc_lang: &str, doc_text: &str, _active_profile: Option<&str>) -> &SpringProjectModel {
        let root = self.resolve_root_for_doc(doc_path);
        self.update_file_lazy(&root, doc_path, doc_lang, doc_text);
        self.projects.get(&root).expect("inserted above")
    }

    pub fn model_for_doc_with_graph(
        &mut self,
        doc_path: &str,
        doc_lang: &str,
        doc_text: &str,
        active_profile: Option<&str>,
    ) -> &SpringProjectModel {
        let root = self.resolve_root_for_doc(doc_path);
        self.update_file_lazy(&root, doc_path, doc_lang, doc_text);
        let model = self.projects.get_mut(&root).expect("inserted above");
        if model.files.get(doc_path).is_some_and(|f| !f.beans.is_empty()) {
            model.ensure_bean_graph(active_profile);
        }
        self.projects.get(&root).expect("inserted above")
    }
}

pub fn should_index_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".java") || lower.ends_with(".kt") || lower.ends_with(".kts") || lower.ends_with(".xml") {
        return true;
    }
    is_spring_config_path(path)
}

pub fn detect_language_from_path(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".kt") || lower.ends_with(".kts") {
        "kotlin"
    } else if lower.ends_with(".java") {
        "java"
    } else if lower.ends_with(".xml") {
        "xml"
    } else if lower.ends_with(".properties") {
        "properties"
    } else {
        "yaml"
    }
}

pub fn parse_file(path: &str, language: &str, text: &str) -> FileFacts {
    let lang = if language.is_empty() {
        detect_language_from_path(path)
    } else {
        language
    };
    if text.len() > MAX_PARSE_BYTES {
        return FileFacts {
            path: path.to_owned(),
            language: lang.to_owned(),
            is_kotlin: lang == "kotlin" || path.ends_with(".kt") || path.ends_with(".kts"),
            ..FileFacts::default()
        };
    }
    if is_spring_config_path(path) || matches!(lang, "yaml" | "yml" | "properties") {
        let cf = parse_config_file(path, text);
        let mut facts = FileFacts {
            path: path.to_owned(),
            language: lang.to_owned(),
            ..FileFacts::default()
        };
        for e in &cf.entries {
            for ph in &e.placeholders {
                facts.value_sites.push(crate::model::ValueExprSite {
                    raw: format!("${{{}}}", ph.key),
                    prop_key: Some(ph.key.clone()),
                    default_value: ph.default_value.clone(),
                    is_spel: false,
                    is_conditional_on_property: false,
                    file: path.to_owned(),
                    key_range: ph.key_range,
                    full_range: ph.full_range,
                    line: ph.key_range.start.line,
                });
            }
        }
        for (target, range) in &cf.imports {
            facts.link_targets.push(crate::model::LinkTargetSite {
                raw_target: target.clone(),
                range: *range,
                file: path.to_owned(),
            });
        }
        facts.config_file = Some(cf);
        facts
    } else if lang == "xml" || path.ends_with(".xml") {
        parse_spring_xml(path, text)
    } else {
        parse_jvm_source(path, lang, text)
    }
}

pub fn scan_project(
    cache: &mut ProjectCache,
    project: &Project,
    active_profile: Option<&str>,
    show_task: bool,
    force: bool,
) {
    let Ok(root) = project.root() else {
        return;
    };
    let mut task = if show_task {
        Some(host::begin_task("Indexing Spring project…", true))
    } else {
        None
    };

    let globs = [
        "**/application*.yml",
        "**/application*.yaml",
        "**/application*.properties",
        "**/bootstrap*.yml",
        "**/bootstrap*.yaml",
        "**/bootstrap*.properties",
        "**/*Context*.xml",
        "**/*context*.xml",
        "**/*beans*.xml",
        "**/*.java",
        "**/*.kt",
    ];

    let mut candidate_paths = BTreeSet::new();
    for g in globs {
        if candidate_paths.len() >= MAX_TOTAL_SCAN_FILES {
            break;
        }
        if let Some(t) = &task
            && t.is_cancelled()
        {
            return;
        }
        if let Ok(found) = project.find_files(g, MAX_SCAN_FILES_PER_GLOB) {
            for p in found {
                if !is_ignored_build_path(&p) {
                    candidate_paths.insert(p);
                    if candidate_paths.len() >= MAX_TOTAL_SCAN_FILES {
                        break;
                    }
                }
            }
        }
    }

    let (mut existing_files, mut existing_hashes) = if !force {
        cache
            .projects
            .get(&root)
            .map(|m| (m.files.clone(), m.file_hashes.clone()))
            .unwrap_or_default()
    } else {
        (BTreeMap::new(), HashMap::new())
    };

    let pending_count = candidate_paths
        .iter()
        .filter(|p| force || !existing_files.contains_key(p.as_str()))
        .count();

    if task.is_none() && pending_count > AUTO_TASK_THRESHOLD {
        task = Some(host::begin_task("Indexing Spring project…", true));
    }

    let total = candidate_paths.len() as u32;
    let mut new_files = BTreeMap::new();
    let mut new_hashes = HashMap::new();
    for (idx_i, p) in candidate_paths.iter().enumerate() {
        if let Some(t) = &task {
            if idx_i % 5 == 0 && t.is_cancelled() {
                let model = cache.get_or_init(&root);
                for (fp, facts) in new_files {
                    model.files.insert(fp, facts);
                }
                for (hp, h) in new_hashes {
                    model.file_hashes.insert(hp, h);
                }
                model.scanned = false;
                model.graph_dirty = true;
                return;
            }
            if idx_i % 10 == 0 {
                t.report(idx_i as u32, Some(total), Some(p));
            }
        }
        if !force
            && let Some(cached_facts) = existing_files.remove(p)
        {
            if let Some(h) = existing_hashes.remove(p) {
                new_hashes.insert(p.clone(), h);
            }
            new_files.insert(p.clone(), cached_facts);
            continue;
        }
        if let Ok(text) = project.read_file(p)
            && text.len() <= MAX_SCAN_FILE_BYTES
        {
            let lang = detect_language_from_path(p);
            new_hashes.insert(p.clone(), content_hash(&text));
            new_files.insert(p.clone(), parse_file(p, lang, &text));
        }
    }

    if let Some(t) = &task
        && t.is_cancelled()
    {
        return;
    }
    let starters = detect_starters(project);
    if let Some(t) = &task
        && t.is_cancelled()
    {
        return;
    }
    let custom_metadata = load_project_metadata(project);

    let model = cache.get_or_init(&root);
    model.files = new_files;
    model.file_hashes = new_hashes;
    model.starters = starters;
    model.custom_metadata = custom_metadata;
    model.scanned = true;
    model.graph_dirty = true;
    model.ensure_bean_graph(active_profile);

    if let Some(t) = &task {
        t.report(total, Some(total), Some("Spring index ready"));
    }
}

fn is_ignored_build_path(path: &str) -> bool {
    path.contains("/target/") || path.contains("/build/") || path.contains("/.git/") || path.contains("/node_modules/")
}

pub fn detect_starters(project: &Project) -> ActiveStarters {
    let mut s = ActiveStarters::default();
    if let Ok(nodes) = project.build_dependencies(None)
        && !nodes.is_empty()
    {
        apply_dependency_nodes(&mut s, &nodes);
        return s;
    }
    // Fallback lightweight pom.xml / build.gradle inspection when build analyzer hasn't synced yet
    for build_file in ["pom.xml", "build.gradle", "build.gradle.kts"] {
        if let Ok(text) = project.read_file(build_file) {
            apply_build_text_heuristics(&mut s, &text);
        }
    }
    s
}

pub fn apply_dependency_nodes(s: &mut ActiveStarters, nodes: &[DependencyNode]) {
    for n in nodes {
        let g = n.group.as_str();
        let a = n.artifact.as_str();
        if g == "org.springframework.boot" && s.spring_boot_version.is_none() && !n.version.is_empty() {
            s.spring_boot_version = Some(n.version.clone());
        }
        if a.contains("spring-boot-starter-webflux") {
            s.webflux = true;
        } else if a.contains("spring-boot-starter-web") || a == "spring-webmvc" {
            s.web = true;
        }
        if a.contains("spring-boot-starter-data-jpa") || a == "spring-data-jpa" {
            s.data_jpa = true;
        }
        if a.contains("spring-boot-starter-data-jdbc") {
            s.data_jdbc = true;
        }
        if a.contains("spring-boot-starter-data-mongodb") {
            s.data_mongo = true;
        }
        if a.contains("spring-boot-starter-data-r2dbc") {
            s.data_r2dbc = true;
        }
        if a.contains("spring-boot-starter-security") || a == "spring-security-core" {
            s.security = true;
        }
        if a.contains("spring-boot-starter-actuator") {
            s.actuator = true;
        }
        if a.contains("spring-boot-starter-validation") {
            s.validation = true;
        }
        if a.contains("spring-boot-starter-data-redis") {
            s.redis = true;
        }
        if a.contains("spring-kafka") {
            s.kafka = true;
        }
        if a.contains("spring-boot-starter-amqp") {
            s.amqp = true;
        }
        if a.contains("openfeign") {
            s.feign = true;
        }
        if g == "org.springframework.cloud" {
            s.cloud = true;
        }
        if a.contains("spring-boot-devtools") {
            s.devtools = true;
        }
        if a.contains("spring-boot-starter-aop") || a == "aspectjweaver" {
            s.aop = true;
        }
        if a.contains("spring-boot-starter-cache") {
            s.cache = true;
        }
    }
}

fn apply_build_text_heuristics(s: &mut ActiveStarters, text: &str) {
    if text.contains("spring-boot-starter-webflux") {
        s.webflux = true;
    }
    if text.contains("spring-boot-starter-web\"")
        || text.contains("spring-boot-starter-web'")
        || text.contains("spring-boot-starter-web<")
    {
        s.web = true;
    }
    if text.contains("spring-boot-starter-data-jpa") {
        s.data_jpa = true;
    }
    if text.contains("spring-boot-starter-data-jdbc") {
        s.data_jdbc = true;
    }
    if text.contains("spring-boot-starter-data-mongodb") {
        s.data_mongo = true;
    }
    if text.contains("spring-boot-starter-data-r2dbc") {
        s.data_r2dbc = true;
    }
    if text.contains("spring-boot-starter-security") {
        s.security = true;
    }
    if text.contains("spring-boot-starter-actuator") {
        s.actuator = true;
    }
    if text.contains("spring-boot-starter-validation") {
        s.validation = true;
    }
    if text.contains("spring-boot-starter-data-redis") {
        s.redis = true;
    }
    if text.contains("spring-kafka") {
        s.kafka = true;
    }
    if text.contains("spring-boot-starter-amqp") {
        s.amqp = true;
    }
    if text.contains("openfeign") {
        s.feign = true;
    }
    if text.contains("spring-boot-devtools") {
        s.devtools = true;
    }
    if text.contains("spring-boot-starter-aop") {
        s.aop = true;
    }
    if text.contains("spring-boot-starter-cache") {
        s.cache = true;
    }
}

fn load_project_metadata(project: &Project) -> Vec<PropMeta> {
    let mut out = Vec::new();
    for g in [
        "**/META-INF/spring-configuration-metadata.json",
        "**/META-INF/additional-spring-configuration-metadata.json",
    ] {
        if let Ok(files) = project.find_files(g, 32) {
            for f in files {
                if let Ok(text) = project.read_file(&f) {
                    out.extend(parse_metadata_json(&text));
                }
            }
        }
    }
    out
}

pub fn bean_matches_profile(bean: &BeanDef, active_profile: Option<&str>) -> bool {
    if bean.profiles.is_empty() {
        return true;
    }
    let active = active_profile.unwrap_or("default");
    bean.profiles.iter().any(|expr| {
        if let Some(neg) = expr.strip_prefix('!') {
            neg != active
        } else {
            expr == active
        }
    })
}

pub fn bean_matches_type(bean: &BeanDef, target_type: &str) -> bool {
    bean.class_name == target_type
        || bean.class_fqn == target_type
        || bean.interfaces.iter().any(|i| i == target_type)
        || bean.superclass.as_deref() == Some(target_type)
        || bean.name == target_type
}

pub struct BeanLookupIndex<'a> {
    by_type: HashMap<&'a str, Vec<&'a BeanDef>>,
    by_id: HashMap<&'a str, &'a BeanDef>,
    has_beans: bool,
}

impl<'a> BeanLookupIndex<'a> {
    pub fn build(model: &'a SpringProjectModel, active_profile: Option<&str>) -> Self {
        let mut by_type: HashMap<&'a str, Vec<&'a BeanDef>> = HashMap::new();
        let mut by_id: HashMap<&'a str, &'a BeanDef> = HashMap::new();
        let mut has_beans = false;

        fn push_unique<'b>(bucket: &mut Vec<&'b BeanDef>, bean: &'b BeanDef) {
            if !bucket.iter().any(|existing| std::ptr::eq(*existing, bean)) {
                bucket.push(bean);
            }
        }

        for b in model.files.values().flat_map(|f| f.beans.iter()) {
            has_beans = true;
            if !bean_matches_profile(b, active_profile) {
                continue;
            }
            by_id.insert(b.id.as_str(), b);
            if !b.class_name.is_empty() {
                push_unique(by_type.entry(b.class_name.as_str()).or_default(), b);
            }
            if !b.class_fqn.is_empty() {
                push_unique(by_type.entry(b.class_fqn.as_str()).or_default(), b);
            }
            for iface in &b.interfaces {
                if !iface.is_empty() {
                    push_unique(by_type.entry(iface.as_str()).or_default(), b);
                }
            }
            if let Some(sup) = b.superclass.as_deref()
                && !sup.is_empty()
            {
                push_unique(by_type.entry(sup).or_default(), b);
            }
            if !b.name.is_empty() {
                push_unique(by_type.entry(b.name.as_str()).or_default(), b);
            }
        }

        Self {
            by_type,
            by_id,
            has_beans,
        }
    }

    pub fn has_beans(&self) -> bool {
        self.has_beans
    }

    pub fn bean_by_id(&self, id: &str) -> Option<&'a BeanDef> {
        self.by_id.get(id).copied()
    }

    pub fn resolve(&self, inj: &InjectionSite) -> (WiringStatus, Vec<ResolvedBeanTarget>) {
        let mut candidates: Vec<&BeanDef> = self
            .by_type
            .get(inj.target_type.as_str())
            .cloned()
            .unwrap_or_default();

        if let Some((q, _)) = &inj.qualifier {
            candidates.retain(|b| b.name == *q || b.qualifiers.iter().any(|(bq, _)| bq == q));
        } else if candidates.len() > 1 {
            let by_name: Vec<&BeanDef> = candidates
                .iter()
                .copied()
                .filter(|b| b.name == inj.field_or_param_name)
                .collect();
            if by_name.len() == 1 {
                candidates = by_name;
            }
        }

        if candidates.is_empty() {
            let status = if !inj.required || inj.is_collection_or_provider {
                WiringStatus::OptionalMissing
            } else {
                WiringStatus::Missing
            };
            return (status, Vec::new());
        }

        let primaries: Vec<&BeanDef> = candidates.iter().copied().filter(|b| b.is_primary).collect();
        if candidates.len() > 1 && primaries.len() == 1 {
            candidates = primaries;
        }

        let targets: Vec<ResolvedBeanTarget> = candidates
            .iter()
            .map(|b| ResolvedBeanTarget {
                bean_id: b.id.clone(),
                bean_name: b.name.clone(),
                class_name: b.class_name.clone(),
                is_primary: b.is_primary,
                file: b.file.clone(),
                range: b.name_range,
            })
            .collect();

        let status = if inj.is_collection_or_provider || targets.len() == 1 {
            WiringStatus::Resolved
        } else {
            WiringStatus::Ambiguous
        };
        (status, targets)
    }
}

pub fn resolve_injection(
    model: &SpringProjectModel,
    inj: &InjectionSite,
    active_profile: Option<&str>,
) -> (WiringStatus, Vec<ResolvedBeanTarget>) {
    let mut candidates: Vec<&BeanDef> = model
        .files
        .values()
        .flat_map(|f| f.beans.iter())
        .filter(|b| bean_matches_profile(b, active_profile) && bean_matches_type(b, &inj.target_type))
        .collect();

    if let Some((q, _)) = &inj.qualifier {
        candidates.retain(|b| {
            b.name == *q || b.qualifiers.iter().any(|(bq, _)| bq == q)
        });
    } else if candidates.len() > 1 {
        // Match by field/param name if exact bean name matches
        let by_name: Vec<&BeanDef> = candidates
            .iter()
            .copied()
            .filter(|b| b.name == inj.field_or_param_name)
            .collect();
        if by_name.len() == 1 {
            candidates = by_name;
        }
    }

    if candidates.is_empty() {
        let status = if !inj.required || inj.is_collection_or_provider {
            WiringStatus::OptionalMissing
        } else {
            WiringStatus::Missing
        };
        return (status, Vec::new());
    }

    let primaries: Vec<&BeanDef> = candidates.iter().copied().filter(|b| b.is_primary).collect();
    if candidates.len() > 1 && primaries.len() == 1 {
        candidates = primaries;
    }

    let targets: Vec<ResolvedBeanTarget> = candidates
        .iter()
        .map(|b| ResolvedBeanTarget {
            bean_id: b.id.clone(),
            bean_name: b.name.clone(),
            class_name: b.class_name.clone(),
            is_primary: b.is_primary,
            file: b.file.clone(),
            range: b.name_range,
        })
        .collect();

    let status = if inj.is_collection_or_provider || targets.len() == 1 {
        WiringStatus::Resolved
    } else {
        WiringStatus::Ambiguous
    };
    (status, targets)
}

impl SpringProjectModel {
    pub fn ensure_bean_graph(&mut self, active_profile: Option<&str>) {
        if !self.graph_dirty && self.graph_profile.as_deref() == active_profile {
            return;
        }
        let (edges, cycles) = compute_bean_graph(self, active_profile);
        self.bean_edges = edges;
        self.cycles = cycles;
        self.graph_dirty = false;
        self.graph_profile = active_profile.map(str::to_owned);
    }
}

pub fn compute_bean_graph(
    model: &SpringProjectModel,
    active_profile: Option<&str>,
) -> (Vec<BeanGraphEdge>, Vec<Vec<String>>) {
    let bean_idx = BeanLookupIndex::build(model, active_profile);
    let mut adj: HashMap<String, Vec<String>> = HashMap::new();
    let mut edges = Vec::new();
    let mut owner_map: HashMap<String, String> = HashMap::new();

    for b in model.files.values().flat_map(|f| f.beans.iter()) {
        if bean_matches_profile(b, active_profile) {
            owner_map.entry(b.name.clone()).or_insert_with(|| b.id.clone());
            owner_map.entry(b.class_name.clone()).or_insert_with(|| b.id.clone());
        }
    }

    for inj in model.files.values().flat_map(|f| f.injections.iter()) {
        let from_id = owner_map
            .get(&inj.owner_bean)
            .or_else(|| owner_map.get(&inj.owner_class))
            .cloned();
        let Some(from_id) = from_id else {
            continue;
        };
        let (_, targets) = bean_idx.resolve(inj);
        for t in targets {
            if t.bean_id == from_id {
                continue;
            }
            adj.entry(from_id.clone()).or_default().push(t.bean_id.clone());
            edges.push(BeanGraphEdge {
                from_id: from_id.clone(),
                to_id: t.bean_id,
                label: inj.field_or_param_name.clone(),
                is_cycle: false,
            });
        }
    }

    // DFS cycle detection
    let mut state: HashMap<String, u8> = HashMap::new(); // 0=unvisited, 1=visiting, 2=done
    let mut stack = Vec::new();
    let mut cycles = Vec::new();
    let mut cycle_edge_pairs = BTreeSet::new();

    let mut nodes: Vec<String> = adj.keys().cloned().collect();
    nodes.sort();
    for node in nodes {
        if !state.contains_key(&node) {
            dfs_cycle(&node, &adj, &mut state, &mut stack, &mut cycles, &mut cycle_edge_pairs);
        }
    }

    for e in &mut edges {
        if cycle_edge_pairs.contains(&(e.from_id.clone(), e.to_id.clone())) {
            e.is_cycle = true;
        }
    }

    (edges, cycles)
}

fn dfs_cycle(
    u: &str,
    adj: &HashMap<String, Vec<String>>,
    state: &mut HashMap<String, u8>,
    stack: &mut Vec<String>,
    cycles: &mut Vec<Vec<String>>,
    cycle_edges: &mut BTreeSet<(String, String)>,
) {
    state.insert(u.to_owned(), 1);
    stack.push(u.to_owned());
    if let Some(neighbors) = adj.get(u) {
        for v in neighbors {
            match state.get(v).copied().unwrap_or(0) {
                0 => dfs_cycle(v, adj, state, stack, cycles, cycle_edges),
                1 => {
                    if let Some(pos) = stack.iter().position(|x| x == v) {
                        let cyc: Vec<String> = stack[pos..].to_vec();
                        for w in cyc.windows(2) {
                            cycle_edges.insert((w[0].clone(), w[1].clone()));
                        }
                        if let Some(last) = cyc.last() {
                            cycle_edges.insert((last.clone(), cyc[0].clone()));
                        }
                        if cycles.len() < 32 {
                            cycles.push(cyc);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    stack.pop();
    state.insert(u.to_owned(), 2);
}

/// Builds `DocumentIndexer` edges for cross-file navigation, Find Usages & Rename, and Gutter Markers.
/// Deliberately omits Java `@Value` <-> config edges already handled by OpenIDEA's built-in JDT configuration bridge.
pub fn build_document_index_edges(doc: &IndexDoc, root: &str, cache: &mut ProjectCache) -> Edges {
    if !should_index_path(&doc.path) {
        return Edges::default();
    }
    let lang = detect_language_from_path(&doc.path);
    let hash = content_hash(&doc.text);
    let cached_hit = cache
        .projects
        .get(root)
        .is_some_and(|m| m.file_hashes.get(&doc.path) == Some(&hash) && m.files.contains_key(&doc.path));
    if !cached_hit {
        let parsed = parse_file(&doc.path, lang, &doc.text);
        cache.insert_file_facts(root, &doc.path, hash, parsed);
    }
    let Some(facts) = cache.projects.get(root).and_then(|m| m.files.get(&doc.path)) else {
        return Edges::default();
    };

    let mut edges = Edges::default();

    // 1. Beans & interfaces -> bean-def#<Type>, bean-def#<name>, and bean-inject# markers
    for b in &facts.beans {
        edges.definitions.push(Anchor {
            key: format!("bean-def#{}", b.class_name),
            range: b.name_range,
        });
        if b.name != b.class_name {
            edges.definitions.push(Anchor {
                key: format!("bean-def#{}", b.name),
                range: b.name_range,
            });
        }
        for iface in &b.interfaces {
            edges.aliases.push(Alias {
                prefix: format!("bean-def#{iface}#"),
                target: format!("bean-def#{}#", b.class_name),
            });
            edges.definitions.push(Anchor {
                key: format!("bean-def#{iface}"),
                range: b.name_range,
            });
        }
        for (q, q_range) in &b.qualifiers {
            edges.definitions.push(Anchor {
                key: format!("qualifier#{q}"),
                range: *q_range,
            });
            edges.definitions.push(Anchor {
                key: format!("bean-def#qual:{q}"),
                range: b.name_range,
            });
        }
        // Gutter icon on Bean definition to navigate to injection sites
        edges.markers.push(Anchor {
            key: format!("bean-inject#{}", b.class_name),
            range: b.name_range,
        });
    }

    // 2. Injection sites -> bean-inject#<target> definitions, bean-def#<target> refs & markers, qualifier# refs
    for inj in &facts.injections {
        edges.definitions.push(Anchor {
            key: format!("bean-inject#{}", inj.target_type),
            range: inj.range,
        });
        let target_key = if let Some((q, qr)) = &inj.qualifier {
            edges.references.push(Anchor {
                key: format!("qualifier#{q}"),
                range: *qr,
            });
            format!("bean-def#qual:{q}")
        } else {
            format!("bean-def#{}", inj.target_type)
        };
        edges.references.push(Anchor {
            key: target_key.clone(),
            range: inj.type_range,
        });
        edges.markers.push(Anchor {
            key: target_key,
            range: inj.range,
        });
    }

    // 3. Spring Events -> bidirectional publisher <-> listener
    for ev in &facts.event_sites {
        match ev.kind {
            EventKind::Publisher => {
                edges.definitions.push(Anchor {
                    key: format!("event-pub#{}", ev.event_type),
                    range: ev.range,
                });
                edges.markers.push(Anchor {
                    key: format!("event-listener#{}", ev.event_type),
                    range: ev.range,
                });
            }
            EventKind::Listener => {
                edges.definitions.push(Anchor {
                    key: format!("event-listener#{}", ev.event_type),
                    range: ev.range,
                });
                edges.markers.push(Anchor {
                    key: format!("event-pub#{}", ev.event_type),
                    range: ev.range,
                });
            }
        }
    }

    // 4. Config properties (ONLY Kotlin + @ConditionalOnProperty + config declarations for Kotlin/ConditionalOnProperty)
    if let Some(cf) = &facts.config_file {
        for e in &cf.entries {
            edges.definitions.push(Anchor {
                key: format!("prop#{}", e.relaxed_key),
                range: e.key_range,
            });
        }
    }
    for v in &facts.value_sites {
        if let Some(k) = &v.prop_key
            && (facts.is_kotlin || v.is_conditional_on_property)
        {
            let rk = normalize_relaxed_key(k);
            edges.references.push(Anchor {
                key: format!("prop#{rk}"),
                range: v.key_range,
            });
            edges.markers.push(Anchor {
                key: format!("prop#{rk}"),
                range: v.key_range,
            });
        }
    }

    // Enforce WIT DocumentIndexer edge bounds (max 4,096 each, 256 aliases)
    edges.definitions.truncate(4096);
    edges.references.truncate(4096);
    edges.markers.truncate(4096);
    edges.aliases.truncate(256);

    edges
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_beans_primary_qualifier_and_detects_cycles() {
        let mut cache = ProjectCache::default();
        cache.update_file(
            "/p",
            "/p/Repo.java",
            "java",
            "public interface Repo {}\n@Repository @Primary public class JpaRepo implements Repo {}\n@Repository @Qualifier(\"cache\") public class CacheRepo implements Repo {}",
            None,
        );
        cache.update_file(
            "/p",
            "/p/ServiceA.java",
            "java",
            "@Service public class ServiceA { @Autowired Repo repo; @Autowired ServiceB b; }",
            None,
        );
        cache.update_file(
            "/p",
            "/p/ServiceB.java",
            "java",
            "@Service public class ServiceB { @Autowired ServiceA a; }",
            None,
        );
        let model = cache.projects.get("/p").unwrap();
        assert!(!model.cycles.is_empty(), "A <-> B cycle should be detected");

        let a_injs: Vec<_> = model
            .files
            .get("/p/ServiceA.java")
            .unwrap()
            .injections
            .iter()
            .collect();
        let (st, targets) = resolve_injection(model, a_injs[0], None);
        assert_eq!(st, WiringStatus::Resolved);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].class_name, "JpaRepo"); // @Primary wins!
    }

    #[test]
    fn bean_graph_scales_linearly_with_hundreds_of_beans_and_injections() {
        let mut cache = ProjectCache::default();
        let mut model = SpringProjectModel {
            root: "/big".into(),
            ..SpringProjectModel::default()
        };
        for i in 0..300usize {
            let path = format!("/big/Svc{i}.java");
            let next1 = (i + 1) % 300;
            let next2 = (i + 2) % 300;
            let next3 = (i + 3) % 300;
            let src = format!(
                "@Service\npublic class Svc{i} {{\n    @Autowired Svc{next1} a;\n    @Autowired Svc{next2} b;\n    @Autowired Svc{next3} c;\n}}\n"
            );
            let facts = parse_file(&path, "java", &src);
            model.file_hashes.insert(path.clone(), content_hash(&src));
            model.files.insert(path, facts);
        }
        cache.projects.insert("/big".into(), model);
        let m = cache.projects.get_mut("/big").unwrap();
        let (edges, cycles) = compute_bean_graph(m, None);
        assert_eq!(edges.len(), 900);
        assert!(!cycles.is_empty());
    }

    #[test]
    fn large_file_guard_and_non_spring_prefilter() {
        let huge = format!("@Service\npublic class Huge {{\n{}\n}}\n", "int x = 1;\n".repeat(120_000));
        assert!(huge.len() > MAX_PARSE_BYTES);
        let huge_facts = parse_file("/p/Huge.java", "java", &huge);
        assert!(huge_facts.beans.is_empty());

        let pojo = "package com.example;\npublic class UserDto {\n    private String name;\n    public String getName() { return name; }\n}\n";
        let pojo_facts = parse_file("/p/UserDto.java", "java", pojo);
        assert!(pojo_facts.beans.is_empty());
        assert!(pojo_facts.injections.is_empty());
        assert!(pojo_facts.endpoints.is_empty());

        let spring_svc = "@Service\npublic class MySvc {}\n";
        assert_eq!(parse_file("/p/MySvc.java", "java", spring_svc).beans.len(), 1);

        let spring_repo = "public interface MyRepo extends JpaRepository<User, Long> {}\n";
        assert_eq!(parse_file("/p/MyRepo.java", "java", spring_repo).repositories.len(), 1);

        let webflux = "class R { RouterFunction<?> f() { return route().GET(\"/hi\", r -> null).build(); } }\n";
        assert_eq!(parse_file("/p/R.java", "java", webflux).endpoints.len(), 1);
    }
}
