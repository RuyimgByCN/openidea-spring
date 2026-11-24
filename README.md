# Spring & Spring Boot for OpenIDEA (`ruyimg.spring`)

IntelliJ IDEA-aligned Spring & Spring Boot developer suite for OpenIDEA, designed specifically to **complement OpenIDEA's built-in Spring support with zero duplicate features**.

## Architecture & Deduplication vs. OpenIDEA Built-ins

| Area | OpenIDEA Built-in (`openidea`) | Provided by `openidea-spring` (`ruyimg.spring`) |
| --- | --- | --- |
| **Project Scaffold** | Spring Initializr project generator (`ide-scaffold`) | Starter detection from Maven/Gradle graphs (`pom.xml` / `build.gradle*`) for context-aware inspections and panels |
| **Run / Debug & Icons** | Spring Boot main-class detector, JVM run/debug, Spring config & boot file icons | Services Tool Window (`location = "services"`), multi-tab instance Details, ANSI-free Run/Debug console highlight rules (`ConsoleRule`), and 9 Spring File Templates + Live Templates (`snippets/*.json`) |
| **Spring Core & DI** | — | Cross-file `@Component` / `@Service` / `@Repository` / `@Controller` / `@Configuration` / `@Bean` / XML bean index, `@Primary` / `@Qualifier` / `@Profile` / `@Conditional*` resolution, gutter markers, `Find Usages` / `Rename` aliases via `DocumentIndexer`, interactive SVG **Bean Dependency Graph**, circular dependency detection, `@Autowired` Field → Constructor Injection refactoring, and `@Transactional` / `@Async` proxy self-invocation / visibility checks |
| **Boot Config & Profiles** | Java `@Value` / `Environment.getProperty` ↔ YAML/Properties navigation | 180+ curated Boot metadata + project `spring-configuration-metadata.json` + `@ConfigurationProperties` prefix/field completion, rich hover docs, effective property inlay hints, Status Bar active profile menu, Native Profile Diff viewer, bidirectional YAML ↔ `.properties` converter, Kotlin `@Value` / `@ConditionalOnProperty` navigation |
| **Web & Endpoints** | Spring MVC `@RestController` / `@Controller` route index, Search Everywhere `routes`, MVC HTTP Client gutter, `@FeignClient` ↔ Controller, Thymeleaf | WebFlux functional routes (`RouterFunctions` / Kotlin `router {}`) and Spring 6 `@HttpExchange` in `EndpointsProvider`, Unified Endpoints Explorer (`MVC` + `WebFlux` + `@HttpExchange` + `@FeignClient`), `@PathVariable` & duplicate-route diagnostics, URL / cURL copy actions |
| **Data, SpEL & Cron** | — | JPA `@Entity` / `@Table` inspector, Spring Data derived method query validation & JPQL preview, `@Query` / `@NativeQuery` SQL language injection + parameter checks, one-click Datasource sync to OpenIDEA Database tool window, SpEL validator & `@bean` navigation, 6-field Spring `@Scheduled` Cron explainer + inlay hints |
| **Runtime & Actuator** | — | Live Actuator polling (`/health`, `/beans`, `/mappings`, masked `/env`, `/conditions`, `/loggers` level switching) plus optional pure-Java **Runtime Probe Agent** (`agent/openidea-spring-agent.jar`) |

## Capabilities & Security Model

### Required Capabilities
- `project.read`, `editor.read`, `editor.write`, `build.read`, `storage`, `events`, `clipboard`

### Optional Capabilities
- `net:127.0.0.1`, `net:localhost`, `net:ask` — polling local or user-configured Actuator / probe endpoints
- `database.sources` — one-click syncing of `spring.datasource.*` connection details (and keychain password via `set_data_source_credential`) into OpenIDEA's Database Tool Window
- `java.agent` *(auto-derived from `[[contributes.javaAgents]]`)* — attaches `agent/openidea-spring-agent.jar` when enabled by the user

### Pure-Java Runtime Probe Agent Security
The optional probe agent (`agent-src/`) binds strictly to `127.0.0.1` on ports `47391–47398`, installs no bytecode transformer (only reflects Spring `ApplicationContext` on-demand via HTTP), masks sensitive environment/config keys (`password`, `secret`, `token`, `apikey`, `credential`, `privatekey`, JDBC userinfo) before serialization, and requires explicit `X-OpenIDEA-Spring: 1` header checks on write routes such as logger updates.

## Repository Structure

```text
openidea-spring/
├── plugin.toml              # Plugin manifest, commands, views, providers, snippets, templates
├── settings.schema.json     # Plugin settings schema
├── Cargo.toml               # Rust WASI Preview 2 plugin crate (`openidea-plugin` v0.1.50)
├── src/                     # Rust backend: parsers, index, providers, diagnostics, RPC host
├── web-src/                 # Vite + React + TypeScript WebView UI source (5 tabs + SVG graph)
├── web/                     # Built WebView assets referenced by `plugin.toml`
├── agent-src/               # Pure-Java 8-compatible runtime probe agent source & build.sh
├── agent/                   # Built `openidea-spring-agent.jar`
├── icons/                   # Light/dark SVG icon assets (17 SVGs)
├── snippets/                # Java, Kotlin, YAML, Properties Live Templates
├── templates/               # 9 Spring file templates (`templates/*.txt`)
└── locales/                 # `zh-CN.json` localization bundle
```

## Build, Test & Verify

### 1. Build & Self-Test the Java Probe Agent (requires JDK 8+)
```bash
bash agent-src/build.sh
```

### 2. Test & Build the WebView UI
```bash
cd web-src
pnpm install
pnpm test
pnpm build
```

### 3. Run Rust Unit & `MockHost` Integration Tests
```bash
cargo test
```

### 4. Compile `wasm32-wasip2` Release Module
```bash
cargo build --release --target wasm32-wasip2
mkdir -p dist
cp target/wasm32-wasip2/release/openidea_spring.wasm dist/plugin.wasm
```

### 5. Validate Plugin Package with `openidea-plugin`
```bash
../openidea/target/debug/openidea-plugin check .
```

### 6. Link Locally or Pack `.oiplugin`
```bash
../openidea/target/debug/openidea-plugin link .
../openidea/target/debug/openidea-plugin pack .
```

## License

Apache License 2.0 — see [LICENSE](LICENSE).
