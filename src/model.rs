use openidea_plugin::{Location, Range};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BeanStereotype {
    SpringBootApplication,
    Component,
    Service,
    Repository,
    Controller,
    RestController,
    Configuration,
    BeanMethod,
    ConfigurationProperties,
    FeignClient,
    HttpExchange,
    XmlBean,
}

impl BeanStereotype {
    pub fn label(self) -> &'static str {
        match self {
            Self::SpringBootApplication => "@SpringBootApplication",
            Self::Component => "@Component",
            Self::Service => "@Service",
            Self::Repository => "@Repository",
            Self::Controller => "@Controller",
            Self::RestController => "@RestController",
            Self::Configuration => "@Configuration",
            Self::BeanMethod => "@Bean",
            Self::ConfigurationProperties => "@ConfigurationProperties",
            Self::FeignClient => "@FeignClient",
            Self::HttpExchange => "@HttpExchange",
            Self::XmlBean => "<bean>",
        }
    }

    pub fn short_tag(self) -> &'static str {
        match self {
            Self::SpringBootApplication => "BootApp",
            Self::Component => "Component",
            Self::Service => "Service",
            Self::Repository => "Repository",
            Self::Controller => "Controller",
            Self::RestController => "RestController",
            Self::Configuration => "Configuration",
            Self::BeanMethod => "BeanMethod",
            Self::ConfigurationProperties => "ConfigProps",
            Self::FeignClient => "FeignClient",
            Self::HttpExchange => "HttpExchange",
            Self::XmlBean => "XmlBean",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BeanDef {
    pub id: String,
    pub name: String,
    pub class_name: String,
    pub class_fqn: String,
    pub package: String,
    pub stereotype: BeanStereotype,
    pub scope: String,
    pub is_primary: bool,
    pub is_lazy: bool,
    pub proxy_bean_methods: bool,
    pub qualifiers: Vec<(String, Range)>,
    pub profiles: Vec<String>,
    pub conditions: Vec<String>,
    pub interfaces: Vec<String>,
    pub superclass: Option<String>,
    pub factory_bean: Option<String>,
    pub file: String,
    pub name_range: Range,
    pub decl_range: Range,
    pub line: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InjectionKind {
    Field,
    Constructor,
    LombokConstructor,
    Setter,
    BeanMethodParam,
    XmlRef,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InjectionSite {
    pub owner_bean: String,
    pub owner_class: String,
    pub kind: InjectionKind,
    pub field_or_param_name: String,
    pub raw_type: String,
    pub target_type: String,
    pub qualifier: Option<(String, Range)>,
    pub required: bool,
    pub is_collection_or_provider: bool,
    pub is_final: bool,
    pub file: String,
    pub range: Range,
    pub type_range: Range,
    pub line: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EndpointKind {
    Mvc,
    WebFlux,
    HttpExchange,
    FeignClient,
}

impl EndpointKind {
    pub fn framework_label(self) -> &'static str {
        match self {
            Self::Mvc => "Spring MVC",
            Self::WebFlux => "Spring WebFlux",
            Self::HttpExchange => "Spring HTTP Interface",
            Self::FeignClient => "Spring OpenFeign",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathVarParam {
    pub param_name: String,
    pub var_name: String,
    pub required: bool,
    pub range: Range,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EndpointDef {
    pub methods: Vec<String>,
    pub raw_path: String,
    pub full_path: String,
    pub kind: EndpointKind,
    pub controller: String,
    pub handler: String,
    pub service_name: Option<String>,
    pub path_vars: Vec<PathVarParam>,
    pub query_params: Vec<String>,
    pub has_request_body: bool,
    pub consumes: Option<String>,
    pub produces: Option<String>,
    pub file: String,
    pub range: Range,
    pub line: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConfigFormat {
    Yaml,
    Properties,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaceholderRef {
    pub key: String,
    pub default_value: Option<String>,
    pub key_range: Range,
    pub full_range: Range,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigEntry {
    pub key: String,
    pub relaxed_key: String,
    pub value: String,
    pub raw_value: String,
    pub doc_profile: Option<String>,
    pub key_range: Range,
    pub value_range: Range,
    pub line: u32,
    pub placeholders: Vec<PlaceholderRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigFile {
    pub path: String,
    pub format: ConfigFormat,
    pub profile: Option<String>,
    pub entries: Vec<ConfigEntry>,
    pub duplicate_keys: Vec<(String, Range, u32)>,
    pub imports: Vec<(String, Range)>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigPropField {
    pub name: String,
    pub kebab_name: String,
    pub full_key: String,
    pub type_name: String,
    pub default_value: Option<String>,
    pub range: Range,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigPropClass {
    pub prefix: String,
    pub class_name: String,
    pub file: String,
    pub range: Range,
    pub fields: Vec<ConfigPropField>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValueExprSite {
    pub raw: String,
    pub prop_key: Option<String>,
    pub default_value: Option<String>,
    pub is_spel: bool,
    pub is_conditional_on_property: bool,
    pub file: String,
    pub key_range: Range,
    pub full_range: Range,
    pub line: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityField {
    pub name: String,
    pub type_name: String,
    pub column_name: String,
    pub is_id: bool,
    pub range: Range,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityDef {
    pub name: String,
    pub fqn: String,
    pub table_name: String,
    pub superclass: Option<String>,
    pub fields: Vec<EntityField>,
    pub file: String,
    pub range: Range,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryMethod {
    pub name: String,
    pub return_type: String,
    pub param_names: Vec<String>,
    pub param_types: Vec<String>,
    pub explicit_params: Vec<(String, Range)>,
    pub query: Option<QuerySite>,
    pub range: Range,
    pub name_range: Range,
    pub line: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuerySite {
    pub query: String,
    pub native: bool,
    pub inner_range: Range,
    pub named_params: Vec<(String, Range)>,
    pub positional_max: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryDef {
    pub name: String,
    pub base_interface: String,
    pub entity_type: String,
    pub id_type: String,
    pub methods: Vec<RepositoryMethod>,
    pub file: String,
    pub range: Range,
    pub line: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EventKind {
    Publisher,
    Listener,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventSite {
    pub kind: EventKind,
    pub event_type: String,
    pub method_name: Option<String>,
    pub condition_spel: Option<String>,
    pub async_listener: bool,
    pub transactional: bool,
    pub file: String,
    pub range: Range,
    pub line: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledSite {
    pub method_name: String,
    pub cron: Option<String>,
    pub fixed_rate: Option<String>,
    pub fixed_delay: Option<String>,
    pub zone: Option<String>,
    pub file: String,
    pub cron_range: Option<Range>,
    pub range: Range,
    pub line: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProxyIssueKind {
    PrivateMethod,
    FinalMethod,
    SelfInvocation,
    ProxyBeanMethodsFalseCall,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyIssueSite {
    pub kind: ProxyIssueKind,
    pub annotation: String,
    pub method_name: String,
    pub file: String,
    pub range: Range,
    pub modifier_range: Option<Range>,
    pub line: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpelSite {
    pub expr: String,
    pub bean_refs: Vec<(String, Range)>,
    pub prop_refs: Vec<(String, Range)>,
    pub file: String,
    pub range: Range,
    pub line: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkTargetSite {
    pub raw_target: String,
    pub range: Range,
    pub file: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileRefSite {
    pub profile: String,
    pub range: Range,
    pub file: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileFacts {
    pub path: String,
    pub language: String,
    pub package: String,
    pub is_kotlin: bool,
    pub beans: Vec<BeanDef>,
    pub injections: Vec<InjectionSite>,
    pub endpoints: Vec<EndpointDef>,
    pub config_file: Option<ConfigFile>,
    pub config_prop_classes: Vec<ConfigPropClass>,
    pub value_sites: Vec<ValueExprSite>,
    pub entities: Vec<EntityDef>,
    pub repositories: Vec<RepositoryDef>,
    pub event_sites: Vec<EventSite>,
    pub scheduled_sites: Vec<ScheduledSite>,
    pub proxy_issues: Vec<ProxyIssueSite>,
    pub spel_sites: Vec<SpelSite>,
    pub link_targets: Vec<LinkTargetSite>,
    pub profile_refs: Vec<ProfileRefSite>,
    pub duplicate_bean_methods: Vec<(String, Range)>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveStarters {
    pub spring_boot_version: Option<String>,
    pub web: bool,
    pub webflux: bool,
    pub data_jpa: bool,
    pub data_jdbc: bool,
    pub data_mongo: bool,
    pub data_r2dbc: bool,
    pub security: bool,
    pub actuator: bool,
    pub validation: bool,
    pub redis: bool,
    pub kafka: bool,
    pub amqp: bool,
    pub feign: bool,
    pub cloud: bool,
    pub devtools: bool,
    pub aop: bool,
    pub cache: bool,
    pub scheduling: bool,
}

impl ActiveStarters {
    pub fn badges(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.web {
            out.push("Web");
        }
        if self.webflux {
            out.push("WebFlux");
        }
        if self.data_jpa {
            out.push("Data JPA");
        }
        if self.data_jdbc {
            out.push("Data JDBC");
        }
        if self.data_mongo {
            out.push("MongoDB");
        }
        if self.data_r2dbc {
            out.push("R2DBC");
        }
        if self.security {
            out.push("Security");
        }
        if self.actuator {
            out.push("Actuator");
        }
        if self.validation {
            out.push("Validation");
        }
        if self.redis {
            out.push("Redis");
        }
        if self.kafka {
            out.push("Kafka");
        }
        if self.amqp {
            out.push("RabbitMQ");
        }
        if self.feign {
            out.push("OpenFeign");
        }
        if self.aop {
            out.push("AOP");
        }
        if self.cache {
            out.push("Cache");
        }
        if self.devtools {
            out.push("DevTools");
        }
        out
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BeanGraphEdge {
    pub from_id: String,
    pub to_id: String,
    pub label: String,
    pub is_cycle: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedBeanTarget {
    pub bean_id: String,
    pub bean_name: String,
    pub class_name: String,
    pub is_primary: bool,
    pub file: String,
    pub range: Range,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WiringStatus {
    Resolved,
    Missing,
    Ambiguous,
    OptionalMissing,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectiveProperty {
    pub key: String,
    pub relaxed_key: String,
    pub value: String,
    pub masked_value: String,
    pub source_file: String,
    pub source_profile: String,
    pub line: u32,
    pub key_range: Range,
    pub overridden_from: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct SpringProjectModel {
    pub root: String,
    pub files: BTreeMap<String, FileFacts>,
    pub file_hashes: HashMap<String, u64>,
    pub scanned: bool,
    pub graph_dirty: bool,
    pub graph_profile: Option<String>,
    pub starters: ActiveStarters,
    pub custom_metadata: Vec<crate::boot_metadata::PropMeta>,
    pub bean_edges: Vec<BeanGraphEdge>,
    pub cycles: Vec<Vec<String>>,
}

impl SpringProjectModel {
    pub fn all_beans(&self) -> Vec<&BeanDef> {
        self.files.values().flat_map(|f| f.beans.iter()).collect()
    }

    pub fn all_injections(&self) -> Vec<&InjectionSite> {
        self.files.values().flat_map(|f| f.injections.iter()).collect()
    }

    pub fn all_endpoints(&self) -> Vec<&EndpointDef> {
        self.files.values().flat_map(|f| f.endpoints.iter()).collect()
    }

    pub fn all_config_files(&self) -> Vec<&ConfigFile> {
        self.files.values().filter_map(|f| f.config_file.as_ref()).collect()
    }

    pub fn all_config_prop_classes(&self) -> Vec<&ConfigPropClass> {
        self.files
            .values()
            .flat_map(|f| f.config_prop_classes.iter())
            .collect()
    }

    pub fn all_entities(&self) -> Vec<&EntityDef> {
        self.files.values().flat_map(|f| f.entities.iter()).collect()
    }

    pub fn all_repositories(&self) -> Vec<&RepositoryDef> {
        self.files.values().flat_map(|f| f.repositories.iter()).collect()
    }

    pub fn all_profiles(&self) -> Vec<String> {
        let mut set = BTreeSet::new();
        for cf in self.all_config_files() {
            if let Some(p) = &cf.profile {
                set.insert(p.clone());
            }
            for e in &cf.entries {
                if let Some(p) = &e.doc_profile {
                    for part in p.split(',') {
                        let t = part.trim();
                        if !t.is_empty() && !t.starts_with('!') {
                            set.insert(t.to_owned());
                        }
                    }
                }
            }
        }
        for f in self.files.values() {
            for b in &f.beans {
                for p in &b.profiles {
                    let t = p.trim();
                    if !t.is_empty() && !t.starts_with('!') {
                        set.insert(t.to_owned());
                    }
                }
            }
            for p in &f.profile_refs {
                let t = p.profile.trim();
                if !t.is_empty() && !t.starts_with('!') {
                    set.insert(t.to_owned());
                }
            }
        }
        set.into_iter().collect()
    }

    pub fn to_location(file: &str, range: Range) -> Location {
        Location {
            path: file.to_owned(),
            range,
        }
    }
}
