use crate::model::ConfigPropClass;
use crate::text_util::{levenshtein, normalize_relaxed_key};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PropMeta {
    pub name: String,
    pub type_name: String,
    pub default_value: Option<String>,
    pub description: String,
    pub allowed_values: Vec<String>,
    pub deprecated: bool,
    pub replacement: Option<String>,
    pub group: String,
}

fn p(
    name: &str,
    type_name: &str,
    default_value: Option<&str>,
    description: &str,
    allowed_values: &[&str],
) -> PropMeta {
    let group = name.split('.').take(2).collect::<Vec<_>>().join(".");
    PropMeta {
        name: name.to_owned(),
        type_name: type_name.to_owned(),
        default_value: default_value.map(str::to_owned),
        description: description.to_owned(),
        allowed_values: allowed_values.iter().map(|s| (*s).to_owned()).collect(),
        deprecated: false,
        replacement: None,
        group,
    }
}

fn dep(name: &str, type_name: &str, replacement: &str, description: &str) -> PropMeta {
    let group = name.split('.').take(2).collect::<Vec<_>>().join(".");
    PropMeta {
        name: name.to_owned(),
        type_name: type_name.to_owned(),
        default_value: None,
        description: description.to_owned(),
        allowed_values: Vec::new(),
        deprecated: true,
        replacement: Some(replacement.to_owned()),
        group,
    }
}

pub fn builtin_metadata() -> Vec<PropMeta> {
    vec![
        // Server
        p("server.port", "java.lang.Integer", Some("8080"), "Server HTTP port.", &[]),
        p("server.address", "java.net.InetAddress", None, "Network address to which the server should bind.", &[]),
        p("server.shutdown", "org.springframework.boot.web.server.Shutdown", Some("graceful"), "Type of shutdown that the server will support.", &["graceful", "immediate"]),
        p("server.forward-headers-strategy", "org.springframework.boot.autoconfigure.web.ServerProperties$ForwardHeadersStrategy", None, "Strategy for handling X-Forwarded-* headers.", &["native", "framework", "none"]),
        p("server.max-http-request-header-size", "org.springframework.util.unit.DataSize", Some("8KB"), "Maximum size of the HTTP request header.", &[]),
        p("server.servlet.context-path", "java.lang.String", None, "Context path of the application.", &[]),
        p("server.servlet.encoding.charset", "java.nio.charset.Charset", Some("UTF-8"), "Charset of HTTP requests and responses.", &["UTF-8", "ISO-8859-1"]),
        p("server.servlet.encoding.enabled", "java.lang.Boolean", Some("true"), "Whether to enable http encoding support.", &["true", "false"]),
        p("server.servlet.encoding.force", "java.lang.Boolean", Some("false"), "Whether to force encoding to configured charset.", &["true", "false"]),
        p("server.servlet.session.timeout", "java.time.Duration", Some("30m"), "Session timeout.", &[]),
        p("server.compression.enabled", "java.lang.Boolean", Some("false"), "Whether response compression is enabled.", &["true", "false"]),
        p("server.compression.mime-types", "java.lang.String[]", None, "Comma-separated list of MIME types that should be compressed.", &[]),
        p("server.compression.min-response-size", "org.springframework.util.unit.DataSize", Some("2KB"), "Minimum Content-Length value required for compression.", &[]),
        p("server.error.include-message", "org.springframework.boot.autoconfigure.web.ErrorProperties$IncludeAttribute", Some("never"), "When to include a message attribute.", &["never", "always", "on_param"]),
        p("server.error.include-binding-errors", "org.springframework.boot.autoconfigure.web.ErrorProperties$IncludeAttribute", Some("never"), "When to include an errors attribute.", &["never", "always", "on_param"]),
        p("server.error.include-stacktrace", "org.springframework.boot.autoconfigure.web.ErrorProperties$IncludeAttribute", Some("never"), "When to include a stacktrace attribute.", &["never", "always", "on_param"]),
        p("server.error.path", "java.lang.String", Some("/error"), "Path of the error controller.", &[]),
        p("server.ssl.enabled", "java.lang.Boolean", Some("false"), "Whether to enable SSL support.", &["true", "false"]),
        p("server.ssl.key-store", "java.lang.String", None, "Path to the key store that holds the SSL certificate.", &[]),
        p("server.ssl.key-store-password", "java.lang.String", None, "Password used to access the key store.", &[]),
        p("server.ssl.key-store-type", "java.lang.String", None, "Type of the key store (PKCS12, JKS).", &["PKCS12", "JKS"]),
        p("server.ssl.protocol", "java.lang.String", Some("TLS"), "SSL protocol to use.", &["TLS", "TLSv1.2", "TLSv1.3"]),
        p("server.tomcat.threads.max", "java.lang.Integer", Some("200"), "Maximum amount of worker threads.", &[]),
        p("server.tomcat.threads.min-spare", "java.lang.Integer", Some("10"), "Minimum amount of worker threads.", &[]),
        p("server.tomcat.accept-count", "java.lang.Integer", Some("100"), "Maximum queue length for incoming connection requests.", &[]),
        p("server.tomcat.max-connections", "java.lang.Integer", Some("8192"), "Maximum number of connections accepted and processed.", &[]),
        p("server.tomcat.connection-timeout", "java.time.Duration", None, "Time connectors wait after accepting a connection.", &[]),
        p("server.http2.enabled", "java.lang.Boolean", Some("false"), "Whether to enable HTTP/2 support.", &["true", "false"]),
        // Spring Core / Main / Profiles / Config
        p("spring.application.name", "java.lang.String", None, "Application name.", &[]),
        p("spring.application.group", "java.lang.String", None, "Application group.", &[]),
        p("spring.profiles.active", "java.lang.String[]", None, "Comma-separated list of active profiles.", &[]),
        p("spring.profiles.default", "java.lang.String[]", None, "Name of the profile to enable if no profile is active.", &[]),
        p("spring.profiles.include", "java.lang.String[]", None, "Unconditionally activate the specified comma-separated list of profiles.", &[]),
        p("spring.config.import", "java.lang.String[]", None, "Import additional config data locations.", &[]),
        p("spring.config.activate.on-profile", "java.lang.String[]", None, "Profile expression that must match to activate this document.", &[]),
        p("spring.config.location", "java.lang.String[]", None, "Config file locations that replace the defaults.", &[]),
        p("spring.config.additional-location", "java.lang.String[]", None, "Config file locations used in addition to the defaults.", &[]),
        p("spring.main.banner-mode", "org.springframework.boot.Banner$Mode", Some("console"), "Mode used to display the banner when the application runs.", &["console", "log", "off"]),
        p("spring.main.allow-bean-definition-overriding", "java.lang.Boolean", Some("false"), "Whether bean definitions with the same name may override earlier ones.", &["true", "false"]),
        p("spring.main.allow-circular-references", "java.lang.Boolean", Some("false"), "Whether to allow circular references between beans and automatically resolve them.", &["true", "false"]),
        p("spring.main.lazy-initialization", "java.lang.Boolean", Some("false"), "Whether initialization should be performed lazily.", &["true", "false"]),
        p("spring.main.web-application-type", "org.springframework.boot.WebApplicationType", None, "Flag to explicitly request a specific type of web application.", &["none", "servlet", "reactive"]),
        p("spring.main.keep-alive", "java.lang.Boolean", Some("false"), "Whether to keep the application alive even if there are no more non-daemon threads.", &["true", "false"]),
        p("spring.threads.virtual.enabled", "java.lang.Boolean", Some("false"), "Whether to use virtual threads (JDK 21+).", &["true", "false"]),
        p("spring.lifecycle.timeout-per-shutdown-phase", "java.time.Duration", Some("30s"), "Timeout for the shutdown of any phase.", &[]),
        p("spring.output.ansi.enabled", "org.springframework.boot.ansi.AnsiOutput$Enabled", Some("detect"), "Configure the ANSI output.", &["detect", "always", "never"]),
        p("spring.aop.auto", "java.lang.Boolean", Some("true"), "Add @EnableAspectJAutoProxy.", &["true", "false"]),
        p("spring.aop.proxy-target-class", "java.lang.Boolean", Some("true"), "Whether subclass-based (CGLIB) proxies are created.", &["true", "false"]),
        p("spring.messages.basename", "java.lang.String", Some("messages"), "Comma-separated list of basenames for MessageSource.", &[]),
        p("spring.messages.encoding", "java.nio.charset.Charset", Some("UTF-8"), "Message bundles encoding.", &["UTF-8"]),
        // Datasource & Hikari & SQL
        p("spring.datasource.url", "java.lang.String", None, "JDBC URL of the database.", &[]),
        p("spring.datasource.username", "java.lang.String", None, "Login username of the database.", &[]),
        p("spring.datasource.password", "java.lang.String", None, "Login password of the database.", &[]),
        p("spring.datasource.driver-class-name", "java.lang.String", None, "Fully qualified name of the JDBC driver.", &["org.postgresql.Driver", "com.mysql.cj.jdbc.Driver", "org.h2.Driver", "org.sqlite.JDBC", "com.microsoft.sqlserver.jdbc.SQLServerDriver", "oracle.jdbc.OracleDriver"]),
        p("spring.datasource.type", "java.lang.Class", None, "Fully qualified name of the connection pool implementation.", &["com.zaxxer.hikari.HikariDataSource", "org.apache.tomcat.jdbc.pool.DataSource", "org.apache.commons.dbcp2.BasicDataSource"]),
        p("spring.datasource.name", "java.lang.String", None, "Name of the datasource.", &[]),
        p("spring.datasource.hikari.maximum-pool-size", "java.lang.Integer", Some("10"), "Maximum size that the pool is allowed to reach.", &[]),
        p("spring.datasource.hikari.minimum-idle", "java.lang.Integer", Some("10"), "Minimum number of idle connections maintained by HikariCP.", &[]),
        p("spring.datasource.hikari.connection-timeout", "java.lang.Long", Some("30000"), "Maximum milliseconds a client waits for a connection from the pool.", &[]),
        p("spring.datasource.hikari.idle-timeout", "java.lang.Long", Some("600000"), "Maximum time a connection is allowed to sit idle in the pool.", &[]),
        p("spring.datasource.hikari.max-lifetime", "java.lang.Long", Some("1800000"), "Maximum lifetime of a connection in the pool.", &[]),
        p("spring.datasource.hikari.pool-name", "java.lang.String", None, "User-defined name for the connection pool.", &[]),
        p("spring.datasource.hikari.auto-commit", "java.lang.Boolean", Some("true"), "Default auto-commit behavior of connections returned from the pool.", &["true", "false"]),
        p("spring.sql.init.mode", "org.springframework.boot.sql.init.DatabaseInitializationMode", Some("embedded"), "Mode to apply when determining if initialization should be performed.", &["always", "embedded", "never"]),
        p("spring.sql.init.schema-locations", "java.lang.String[]", None, "Locations of the schema (DDL) scripts to apply.", &[]),
        p("spring.sql.init.data-locations", "java.lang.String[]", None, "Locations of the data (DML) scripts to apply.", &[]),
        p("spring.h2.console.enabled", "java.lang.Boolean", Some("false"), "Whether to enable the H2 console.", &["true", "false"]),
        p("spring.h2.console.path", "java.lang.String", Some("/h2-console"), "Path at which the H2 console is available.", &[]),
        // JPA & Hibernate
        p("spring.jpa.show-sql", "java.lang.Boolean", Some("false"), "Whether to enable logging of SQL statements.", &["true", "false"]),
        p("spring.jpa.open-in-view", "java.lang.Boolean", Some("true"), "Register OpenEntityManagerInViewInterceptor.", &["true", "false"]),
        p("spring.jpa.generate-ddl", "java.lang.Boolean", Some("false"), "Whether to initialize the schema on startup.", &["true", "false"]),
        p("spring.jpa.database", "org.springframework.orm.jpa.vendor.Database", None, "Target database to operate on, auto-detected by default.", &["DEFAULT", "POSTGRESQL", "MYSQL", "ORACLE", "SQL_SERVER", "H2", "DB2", "DERBY", "HSQL", "INFORMIX", "SYBASE"]),
        p("spring.jpa.database-platform", "java.lang.String", None, "Name of the target database to operate on.", &[]),
        p("spring.jpa.defer-datasource-initialization", "java.lang.Boolean", Some("false"), "Whether to defer DataSource initialization until after JPA EntityManagerFactory.", &["true", "false"]),
        p("spring.jpa.hibernate.ddl-auto", "java.lang.String", None, "DDL mode.", &["none", "validate", "update", "create", "create-drop"]),
        p("spring.jpa.hibernate.naming.physical-strategy", "java.lang.String", None, "Fully qualified name of the physical naming strategy.", &[]),
        p("spring.jpa.properties.hibernate.format_sql", "java.lang.Boolean", Some("false"), "Format logged SQL statements.", &["true", "false"]),
        p("spring.jpa.properties.hibernate.dialect", "java.lang.String", None, "Hibernate SQL dialect class.", &[]),
        // Flyway & Liquibase
        p("spring.flyway.enabled", "java.lang.Boolean", Some("true"), "Whether to enable Flyway.", &["true", "false"]),
        p("spring.flyway.locations", "java.lang.String[]", Some("classpath:db/migration"), "Locations of migrations scripts.", &[]),
        p("spring.flyway.baseline-on-migrate", "java.lang.Boolean", Some("false"), "Whether to automatically call baseline when migrate is executed.", &["true", "false"]),
        p("spring.liquibase.enabled", "java.lang.Boolean", Some("true"), "Whether to enable Liquibase.", &["true", "false"]),
        p("spring.liquibase.change-log", "java.lang.String", Some("classpath:/db/changelog/db.changelog-master.yaml"), "Change log configuration path.", &[]),
        // Web / MVC / WebFlux / Jackson / Multipart
        p("spring.mvc.servlet.path", "java.lang.String", Some("/"), "Path of the dispatcher servlet.", &[]),
        p("spring.mvc.throw-exception-if-no-handler-found", "java.lang.Boolean", Some("false"), "Whether a NoHandlerFoundException should be thrown if no Handler was found.", &["true", "false"]),
        p("spring.mvc.pathmatch.matching-strategy", "org.springframework.boot.autoconfigure.web.servlet.WebMvcProperties$MatchingStrategy", Some("path_pattern_parser"), "Choice of strategy for matching request paths against mappings.", &["ant_path_matcher", "path_pattern_parser"]),
        p("spring.webflux.base-path", "java.lang.String", None, "Base path for all web handlers.", &[]),
        p("spring.web.resources.static-locations", "java.lang.String[]", None, "Locations of static resources.", &[]),
        p("spring.servlet.multipart.enabled", "java.lang.Boolean", Some("true"), "Whether to enable support of multipart uploads.", &["true", "false"]),
        p("spring.servlet.multipart.max-file-size", "org.springframework.util.unit.DataSize", Some("1MB"), "Max file size.", &[]),
        p("spring.servlet.multipart.max-request-size", "org.springframework.util.unit.DataSize", Some("10MB"), "Max request size.", &[]),
        p("spring.jackson.date-format", "java.lang.String", None, "Date format string or a fully-qualified date format class name.", &[]),
        p("spring.jackson.time-zone", "java.util.TimeZone", None, "Time zone used when formatting dates.", &["UTC", "Asia/Shanghai", "America/New_York", "Europe/London"]),
        p("spring.jackson.default-property-inclusion", "com.fasterxml.jackson.annotation.JsonInclude$Include", None, "Controls the inclusion of properties during serialization.", &["always", "non_null", "non_absent", "non_empty", "non_default"]),
        p("spring.jackson.serialization.write-dates-as-timestamps", "java.lang.Boolean", Some("true"), "Serialize dates as numeric timestamps.", &["true", "false"]),
        p("spring.jackson.deserialization.fail-on-unknown-properties", "java.lang.Boolean", Some("true"), "Fail when unknown JSON properties are encountered.", &["true", "false"]),
        // Security
        p("spring.security.user.name", "java.lang.String", Some("user"), "Default user name.", &[]),
        p("spring.security.user.password", "java.lang.String", None, "Password for the default user name.", &[]),
        p("spring.security.user.roles", "java.util.List", None, "Granted roles for the default user name.", &[]),
        p("spring.security.oauth2.resourceserver.jwt.issuer-uri", "java.lang.String", None, "URI that can either be an OpenID Connect discovery endpoint or an OAuth 2.0 Authorization Server Metadata endpoint.", &[]),
        p("spring.security.oauth2.resourceserver.jwt.jwk-set-uri", "java.lang.String", None, "JSON Web Key URI to use to verify the JWT token.", &[]),
        // Data Redis / Mongo / Cache
        p("spring.data.redis.host", "java.lang.String", Some("localhost"), "Redis server host.", &[]),
        p("spring.data.redis.port", "java.lang.Integer", Some("6379"), "Redis server port.", &[]),
        p("spring.data.redis.password", "java.lang.String", None, "Login password of the redis server.", &[]),
        p("spring.data.redis.username", "java.lang.String", None, "Login username of the redis server.", &[]),
        p("spring.data.redis.database", "java.lang.Integer", Some("0"), "Database index used by the connection factory.", &[]),
        p("spring.data.redis.timeout", "java.time.Duration", None, "Read timeout.", &[]),
        p("spring.data.redis.ssl.enabled", "java.lang.Boolean", Some("false"), "Whether to enable SSL support for Redis.", &["true", "false"]),
        p("spring.data.mongodb.uri", "java.lang.String", Some("mongodb://localhost/test"), "Mongo database URI.", &[]),
        p("spring.data.mongodb.host", "java.lang.String", Some("localhost"), "Mongo server host.", &[]),
        p("spring.data.mongodb.port", "java.lang.Integer", Some("27017"), "Mongo server port.", &[]),
        p("spring.data.mongodb.database", "java.lang.String", Some("test"), "Database name.", &[]),
        p("spring.data.jpa.repositories.enabled", "java.lang.Boolean", Some("true"), "Whether to enable JPA repositories.", &["true", "false"]),
        p("spring.data.jpa.repositories.bootstrap-mode", "org.springframework.data.repository.config.BootstrapMode", Some("default"), "Bootstrap mode for JPA repositories.", &["default", "deferred", "lazy"]),
        p("spring.cache.type", "org.springframework.boot.autoconfigure.cache.CacheType", None, "Cache type. By default, auto-detected according to the environment.", &["generic", "jcache", "ehcache", "hazelcast", "infinispan", "couchbase", "redis", "caffeine", "cache2k", "simple", "none"]),
        p("spring.cache.cache-names", "java.lang.String[]", None, "Comma-separated list of cache names to create.", &[]),
        p("spring.cache.caffeine.spec", "java.lang.String", None, "The spec to use to create caches.", &[]),
        // Messaging & Mail & Task
        p("spring.kafka.bootstrap-servers", "java.lang.String[]", Some("localhost:9092"), "Comma-delimited list of host:port pairs for initial Kafka cluster connection.", &[]),
        p("spring.kafka.consumer.group-id", "java.lang.String", None, "Unique string that identifies the consumer group to which this consumer belongs.", &[]),
        p("spring.kafka.consumer.auto-offset-reset", "java.lang.String", None, "What to do when there is no initial offset in Kafka.", &["earliest", "latest", "none"]),
        p("spring.kafka.producer.acks", "java.lang.String", None, "Number of acknowledgments the producer requires the leader to have received.", &["0", "1", "all"]),
        p("spring.rabbitmq.host", "java.lang.String", Some("localhost"), "RabbitMQ host.", &[]),
        p("spring.rabbitmq.port", "java.lang.Integer", Some("5672"), "RabbitMQ port.", &[]),
        p("spring.rabbitmq.username", "java.lang.String", Some("guest"), "Login user to authenticate to the broker.", &[]),
        p("spring.rabbitmq.password", "java.lang.String", Some("guest"), "Login to authenticate against the broker.", &[]),
        p("spring.rabbitmq.virtual-host", "java.lang.String", None, "Virtual host to use when connecting to the broker.", &[]),
        p("spring.mail.host", "java.lang.String", None, "SMTP server host.", &[]),
        p("spring.mail.port", "java.lang.Integer", None, "SMTP server port.", &[]),
        p("spring.mail.username", "java.lang.String", None, "Login user of the SMTP server.", &[]),
        p("spring.mail.password", "java.lang.String", None, "Login password of the SMTP server.", &[]),
        p("spring.task.execution.pool.core-size", "java.lang.Integer", Some("8"), "Core number of threads.", &[]),
        p("spring.task.execution.pool.max-size", "java.lang.Integer", None, "Maximum allowed number of threads.", &[]),
        p("spring.task.execution.pool.queue-capacity", "java.lang.Integer", None, "Queue capacity.", &[]),
        p("spring.task.execution.thread-name-prefix", "java.lang.String", Some("task-"), "Prefix to use for the names of newly created threads.", &[]),
        p("spring.task.scheduling.pool.size", "java.lang.Integer", Some("1"), "Maximum allowed number of threads for @Scheduled.", &[]),
        p("spring.task.scheduling.thread-name-prefix", "java.lang.String", Some("scheduling-"), "Prefix to use for scheduling thread names.", &[]),
        // DevTools
        p("spring.devtools.restart.enabled", "java.lang.Boolean", Some("true"), "Whether to enable automatic restart.", &["true", "false"]),
        p("spring.devtools.livereload.enabled", "java.lang.Boolean", Some("true"), "Whether to enable a livereload.com compatible server.", &["true", "false"]),
        // Logging
        p("logging.level.root", "java.lang.String", Some("INFO"), "Root logger severity level.", &["TRACE", "DEBUG", "INFO", "WARN", "ERROR", "FATAL", "OFF"]),
        p("logging.level.web", "java.lang.String", None, "Web logger group severity level.", &["TRACE", "DEBUG", "INFO", "WARN", "ERROR", "OFF"]),
        p("logging.level.sql", "java.lang.String", None, "SQL logger group severity level.", &["TRACE", "DEBUG", "INFO", "WARN", "ERROR", "OFF"]),
        p("logging.level.org.springframework", "java.lang.String", None, "Spring Framework package logger level.", &["TRACE", "DEBUG", "INFO", "WARN", "ERROR", "OFF"]),
        p("logging.level.org.hibernate.SQL", "java.lang.String", None, "Hibernate SQL logger level.", &["TRACE", "DEBUG", "INFO", "WARN", "ERROR", "OFF"]),
        p("logging.file.name", "java.lang.String", None, "Log file name (for instance, 'myapp.log').", &[]),
        p("logging.file.path", "java.lang.String", None, "Location of the log file.", &[]),
        p("logging.pattern.console", "java.lang.String", None, "Appender pattern for output to the console.", &[]),
        p("logging.pattern.file", "java.lang.String", None, "Appender pattern for output to a file.", &[]),
        // Management / Actuator
        p("management.server.port", "java.lang.Integer", None, "Management endpoint HTTP port (uses the same port as the application by default).", &[]),
        p("management.server.address", "java.net.InetAddress", None, "Network address to which the management endpoints should bind.", &[]),
        p("management.endpoints.web.base-path", "java.lang.String", Some("/actuator"), "Base path for Web endpoints.", &[]),
        p("management.endpoints.web.exposure.include", "java.lang.String[]", Some("health"), "Endpoint IDs that should be included or '*' for all.", &["*", "health,info", "health,info,metrics,beans,mappings,env,loggers,conditions"]),
        p("management.endpoints.web.exposure.exclude", "java.lang.String[]", None, "Endpoint IDs that should be excluded.", &[]),
        p("management.endpoints.jmx.exposure.include", "java.lang.String[]", Some("*"), "JMX Endpoint IDs that should be included.", &["*"]),
        p("management.endpoint.health.show-details", "org.springframework.boot.actuate.health.ShowDetails", Some("never"), "When to show full health details.", &["never", "when_authorized", "always"]),
        p("management.endpoint.health.show-components", "org.springframework.boot.actuate.health.ShowComponents", None, "When to show components.", &["never", "when_authorized", "always"]),
        p("management.endpoint.health.probes.enabled", "java.lang.Boolean", Some("false"), "Whether to enable liveness and readiness probes.", &["true", "false"]),
        p("management.endpoint.shutdown.enabled", "java.lang.Boolean", Some("false"), "Whether the shutdown endpoint is enabled.", &["true", "false"]),
        p("management.endpoint.env.show-values", "org.springframework.boot.actuate.endpoint.Show", Some("never"), "When to show values from /actuator/env.", &["never", "when_authorized", "always"]),
        p("management.info.env.enabled", "java.lang.Boolean", Some("false"), "Whether to enable info environment contributor.", &["true", "false"]),
        p("management.metrics.tags.application", "java.lang.String", None, "Common application tag on metrics.", &[]),
        p("management.prometheus.metrics.export.enabled", "java.lang.Boolean", Some("true"), "Whether exporting metrics to Prometheus is enabled.", &["true", "false"]),
        p("management.tracing.sampling.probability", "java.lang.Float", Some("0.1"), "Probability in the range from 0.0 to 1.0 that a trace will be sampled.", &[]),
        // Deprecated Spring Boot 2 -> 3/4 properties
        dep("spring.profiles", "java.lang.String[]", "spring.config.activate.on-profile", "Deprecated in Spring Boot 2.4+; use spring.config.activate.on-profile."),
        dep("spring.redis.host", "java.lang.String", "spring.data.redis.host", "Deprecated in Spring Boot 3.0+; moved to spring.data.redis.host."),
        dep("spring.redis.port", "java.lang.Integer", "spring.data.redis.port", "Deprecated in Spring Boot 3.0+; moved to spring.data.redis.port."),
        dep("spring.redis.password", "java.lang.String", "spring.data.redis.password", "Deprecated in Spring Boot 3.0+; moved to spring.data.redis.password."),
        dep("spring.redis.database", "java.lang.Integer", "spring.data.redis.database", "Deprecated in Spring Boot 3.0+; moved to spring.data.redis.database."),
        dep("server.context-path", "java.lang.String", "server.servlet.context-path", "Deprecated in Spring Boot 2.0+; moved to server.servlet.context-path."),
        dep("spring.datasource.schema", "java.lang.String[]", "spring.sql.init.schema-locations", "Deprecated in Spring Boot 2.5+; moved to spring.sql.init.schema-locations."),
        dep("spring.datasource.data", "java.lang.String[]", "spring.sql.init.data-locations", "Deprecated in Spring Boot 2.5+; moved to spring.sql.init.data-locations."),
        dep("spring.datasource.initialization-mode", "java.lang.String", "spring.sql.init.mode", "Deprecated in Spring Boot 2.5+; moved to spring.sql.init.mode."),
        dep("logging.file", "java.lang.String", "logging.file.name", "Deprecated in Spring Boot 2.2+; moved to logging.file.name."),
        dep("logging.path", "java.lang.String", "logging.file.path", "Deprecated in Spring Boot 2.2+; moved to logging.file.path."),
        dep("spring.resources.static-locations", "java.lang.String[]", "spring.web.resources.static-locations", "Deprecated in Spring Boot 2.4+; moved to spring.web.resources.static-locations."),
        dep("management.metrics.export.prometheus.enabled", "java.lang.Boolean", "management.prometheus.metrics.export.enabled", "Deprecated in Spring Boot 3.0+; moved to management.prometheus.metrics.export.enabled."),
        dep("spring.mvc.favicon.enabled", "java.lang.Boolean", "spring.web.resources.add-mappings", "Removed in Spring Boot 2.2+."),
    ]
}

/// Parses `META-INF/spring-configuration-metadata.json` or `additional-spring-configuration-metadata.json`.
pub fn parse_metadata_json(text: &str) -> Vec<PropMeta> {
    let Ok(val) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut hints: HashMap<String, Vec<String>> = HashMap::new();
    if let Some(hint_arr) = val.get("hints").and_then(Value::as_array) {
        for h in hint_arr {
            let Some(name) = h.get("name").and_then(Value::as_str) else {
                continue;
            };
            let mut vals = Vec::new();
            if let Some(items) = h.get("values").and_then(Value::as_array) {
                for item in items {
                    if let Some(v) = item.get("value") {
                        match v {
                            Value::String(s) => vals.push(s.clone()),
                            Value::Number(n) => vals.push(n.to_string()),
                            Value::Bool(b) => vals.push(b.to_string()),
                            _ => {}
                        }
                    }
                }
            }
            if !vals.is_empty() {
                hints.insert(name.to_owned(), vals);
            }
        }
    }

    if let Some(props) = val.get("properties").and_then(Value::as_array) {
        for item in props {
            let Some(name) = item.get("name").and_then(Value::as_str) else {
                continue;
            };
            let type_name = item
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("java.lang.String")
                .to_owned();
            let description = item
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let default_value = item.get("defaultValue").and_then(|v| match v {
                Value::String(s) => Some(s.clone()),
                Value::Number(n) => Some(n.to_string()),
                Value::Bool(b) => Some(b.to_string()),
                _ => None,
            });
            let dep_obj = item.get("deprecation");
            let deprecated = dep_obj.is_some();
            let replacement = dep_obj
                .and_then(|d| d.get("replacement"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            let allowed_values = hints.remove(name).unwrap_or_default();
            let group = name.split('.').take(2).collect::<Vec<_>>().join(".");
            out.push(PropMeta {
                name: name.to_owned(),
                type_name,
                default_value,
                description,
                allowed_values,
                deprecated,
                replacement,
                group,
            });
        }
    }
    out
}

pub struct MetadataIndex {
    by_relaxed: BTreeMap<String, PropMeta>,
    by_name: BTreeMap<String, PropMeta>,
}

impl MetadataIndex {
    pub fn build(custom: &[PropMeta], classes: &[&ConfigPropClass]) -> Self {
        let mut by_relaxed = BTreeMap::new();
        let mut by_name = BTreeMap::new();
        for item in builtin_metadata() {
            by_relaxed.insert(normalize_relaxed_key(&item.name), item.clone());
            by_name.insert(item.name.clone(), item);
        }
        for item in custom {
            by_relaxed.insert(normalize_relaxed_key(&item.name), item.clone());
            by_name.insert(item.name.clone(), item.clone());
        }
        for cls in classes {
            for field in &cls.fields {
                let allowed = if field.type_name == "boolean" || field.type_name == "Boolean" {
                    vec!["true".to_owned(), "false".to_owned()]
                } else {
                    Vec::new()
                };
                let item = PropMeta {
                    name: field.full_key.clone(),
                    type_name: field.type_name.clone(),
                    default_value: field.default_value.clone(),
                    description: format!("Bound to `{}`.`{}`", cls.class_name, field.name),
                    allowed_values: allowed,
                    deprecated: false,
                    replacement: None,
                    group: cls.prefix.clone(),
                };
                by_relaxed.insert(normalize_relaxed_key(&item.name), item.clone());
                by_name.insert(item.name.clone(), item);
            }
        }
        Self { by_relaxed, by_name }
    }

    pub fn get(&self, key: &str) -> Option<&PropMeta> {
        self.by_name
            .get(key)
            .or_else(|| self.by_relaxed.get(&normalize_relaxed_key(key)))
    }

    pub fn is_known_dynamic_prefix(key: &str) -> bool {
        key.starts_with("logging.level.")
            || key.starts_with("logging.group.")
            || key.starts_with("spring.jpa.properties.")
            || key.starts_with("spring.datasource.hikari.")
            || key.starts_with("management.endpoint.")
            || key.starts_with("spring.cloud.")
            || key.starts_with("spring.security.oauth2.")
            || key.starts_with("feign.")
            || key.starts_with("spring.cloud.openfeign.")
    }

    pub fn suggest_similar(&self, key: &str) -> Option<&PropMeta> {
        let target = normalize_relaxed_key(key);
        let mut best: Option<(usize, &PropMeta)> = None;
        for (rk, meta) in &self.by_relaxed {
            let d = levenshtein(&target, rk);
            if d <= 3 && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, meta));
            }
        }
        best.map(|(_, m)| m)
    }

    pub fn complete(&self, prefix: &str, limit: usize) -> Vec<&PropMeta> {
        let norm_prefix = prefix.trim().to_ascii_lowercase();
        let mut out = Vec::new();
        for meta in self.by_name.values() {
            if norm_prefix.is_empty()
                || meta.name.to_ascii_lowercase().starts_with(&norm_prefix)
                || meta.name.to_ascii_lowercase().contains(&norm_prefix)
            {
                out.push(meta);
                if out.len() >= limit {
                    break;
                }
            }
        }
        out
    }

    pub fn all(&self) -> impl Iterator<Item = &PropMeta> {
        self.by_name.values()
    }
}

pub fn validate_property_value(meta: &PropMeta, value: &str) -> Option<String> {
    let v = value.trim();
    if v.is_empty() || v.contains("${") || v.starts_with('@') {
        return None;
    }
    let ty = meta.type_name.as_str();
    if (ty.ends_with("Boolean") || ty == "boolean") && !matches!(v.to_ascii_lowercase().as_str(), "true" | "false") {
        return Some(format!("Expected boolean (`true` or `false`) for `{}`", meta.name));
    }
    if (ty.ends_with("Integer") || ty == "int" || ty.ends_with("Long") || ty == "long") && v.parse::<i64>().is_err() {
        return Some(format!("Expected integer value for `{}` (got `{v}`)", meta.name));
    }
    if meta.name == "server.port" || meta.name == "management.server.port" {
        if let Ok(port) = v.parse::<i64>()
            && !(0..=65535).contains(&port)
        {
            return Some(format!("Port `{port}` is out of range (0..=65535)"));
        }
    }
    if !meta.allowed_values.is_empty()
        && !meta.name.ends_with("exposure.include")
        && !meta
            .allowed_values
            .iter()
            .any(|a| a.eq_ignore_ascii_case(v))
    {
        return Some(format!(
            "Invalid value `{v}` for `{}`; expected one of: {}",
            meta.name,
            meta.allowed_values.join(", ")
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_lookup_handles_relaxed_binding_and_deprecations() {
        let idx = MetadataIndex::build(&[], &[]);
        let cp = idx.get("server.servlet.contextPath").expect("relaxed match");
        assert_eq!(cp.name, "server.servlet.context-path");

        let dep = idx.get("spring.redis.host").expect("deprecated key");
        assert!(dep.deprecated);
        assert_eq!(dep.replacement.as_deref(), Some("spring.data.redis.host"));

        let port = idx.get("server.port").unwrap();
        assert!(validate_property_value(port, "abc").is_some());
        assert!(validate_property_value(port, "99999").is_some());
        assert!(validate_property_value(port, "8080").is_none());
        assert!(validate_property_value(port, "${PORT:8080}").is_none());
    }

    #[test]
    fn parses_spring_configuration_metadata_json() {
        let json = r#"{
          "properties": [
            {
              "name": "app.feature.mode",
              "type": "java.lang.String",
              "description": "Feature mode",
              "defaultValue": "fast"
            },
            {
              "name": "app.feature.old",
              "type": "java.lang.Boolean",
              "deprecation": { "replacement": "app.feature.enabled" }
            }
          ],
          "hints": [
            {
              "name": "app.feature.mode",
              "values": [{ "value": "fast" }, { "value": "safe" }]
            }
          ]
        }"#;
        let parsed = parse_metadata_json(json);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].allowed_values, ["fast", "safe"]);
        assert!(parsed[1].deprecated);
        assert_eq!(parsed[1].replacement.as_deref(), Some("app.feature.enabled"));
    }
}
