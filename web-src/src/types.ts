export type TabId = "beans" | "endpoints" | "config" | "data" | "runtime";

export type WiringStatus = "resolved" | "missing" | "ambiguous" | "optionalMissing";

export interface BeanDependency {
  field: string;
  targetType: string;
  kind: string;
  status: WiringStatus;
  qualifier: string | null;
  resolvedIds: string[];
}

export interface BeanItem {
  id: string;
  name: string;
  className: string;
  classFqn: string;
  package: string;
  stereotype: string;
  stereotypeLabel: string;
  scope: string;
  isPrimary: boolean;
  isLazy: boolean;
  profiles: string[];
  qualifiers: string[];
  file: string;
  line: number;
  dependencies: BeanDependency[];
}

export interface BeanGraphEdge {
  fromId: string;
  toId: string;
  label: string;
  isCycle: boolean;
}

export interface UnifiedEndpointItem {
  id: string;
  methods: string[];
  rawPath: string;
  fullPath: string;
  resolvedUrl: string;
  kind: "mvc" | "webFlux" | "httpExchange" | "feignClient";
  framework: string;
  controller: string;
  handler: string;
  serviceName: string | null;
  queryParams: string[];
  hasRequestBody: boolean;
  duplicateConflict: boolean;
  runtimeMatched: boolean;
  file: string;
  line: number;
}

export interface ConfigFileSummary {
  path: string;
  profile: string | null;
  format: string;
  entryCount: number;
}

export interface EffectivePropertyItem {
  key: string;
  relaxedKey: string;
  maskedValue: string;
  sourceFile: string;
  sourceProfile: string;
  line: number;
  overriddenFrom: string[];
}

export interface ConfigPropFieldItem {
  name: string;
  fullKey: string;
  typeName: string;
  defaultValue: string | null;
  line: number;
}

export interface ConfigPropClassItem {
  prefix: string;
  className: string;
  file: string;
  line: number;
  fields: ConfigPropFieldItem[];
}

export interface EntityFieldItem {
  name: string;
  typeName: string;
  columnName: string;
  isId: boolean;
}

export interface EntityItem {
  name: string;
  fqn: string;
  tableName: string;
  file: string;
  line: number;
  fields: EntityFieldItem[];
}

export interface RepositoryMethodItem {
  name: string;
  returnType: string;
  line: number;
  query: { text: string; native: boolean } | null;
  jpqlPreview: string | null;
  valid: boolean;
  errors: string[];
}

export interface RepositoryItem {
  name: string;
  baseInterface: string;
  entityType: string;
  idType: string;
  file: string;
  line: number;
  methods: RepositoryMethodItem[];
}

export interface ScheduledSiteItem {
  methodName: string;
  cron: string | null;
  fixedRate: string | null;
  fixedDelay: string | null;
  zone: string | null;
  summary: string | null;
  valid: boolean;
  error: string | null;
  file: string;
  line: number;
}

export interface SpelSiteItem {
  expr: string;
  referencedBeans: string[];
  valid: boolean;
  summary: string;
  error: string | null;
  file: string;
  line: number;
}

export interface RuntimeAppInstance {
  id: string;
  source: "agent" | "actuator" | string;
  baseUrl: string;
  appName: string;
  pid: number | null;
  serverPort: number | null;
  contextPath: string;
  activeProfiles: string[];
  ready: boolean;
  healthStatus: string;
  projectRoot: string | null;
  uptimeMs: number | null;
  liveBeansJson: string | null;
  mappingsJson: string | null;
  envJson: string | null;
  conditionsJson: string | null;
  metricsJson: string | null;
  loggersJson: string | null;
  healthJson: string | null;
}

export interface ProjectSnapshot {
  projectRoot: string;
  activeProfile: string | null;
  discoveredProfiles: string[];
  locale: string;
  consoleHighlighting: boolean;
  starters: {
    badges?: string[];
    [key: string]: unknown;
  };
  beans: BeanItem[];
  beanEdges: BeanGraphEdge[];
  cycles: string[][];
  endpoints: UnifiedEndpointItem[];
  configFiles: ConfigFileSummary[];
  effectiveProperties: EffectivePropertyItem[];
  configPropClasses: ConfigPropClassItem[];
  entities: EntityItem[];
  repositories: RepositoryItem[];
  scheduledSites: ScheduledSiteItem[];
  spelSites: SpelSiteItem[];
  runtimeInstances: RuntimeAppInstance[];
}

export interface InitPayload {
  op: "init";
  locale: string;
  pending: { tab?: string; extra?: Record<string, unknown> } | null;
  snapshot: ProjectSnapshot;
}

export interface CronExplainResult {
  valid: boolean;
  summaryEn: string;
  summaryZh: string;
  fields: [string, string][];
  error: string | null;
}

export interface SpelValidateResult {
  valid: boolean;
  summary: string;
  error: string | null;
  beanRefs: string[];
  propRefs: string[];
}

export const DEMO_SNAPSHOT: ProjectSnapshot = {
  projectRoot: "/demo/spring-shop",
  activeProfile: "dev",
  discoveredProfiles: ["default", "dev", "prod"],
  locale: "en",
  consoleHighlighting: true,
  starters: { badges: ["Web", "WebFlux", "Data JPA", "Actuator", "Security"] },
  beans: [
    {
      id: "DemoApp#1",
      name: "demoApplication",
      className: "DemoApplication",
      classFqn: "com.example.DemoApplication",
      package: "com.example",
      stereotype: "springBootApplication",
      stereotypeLabel: "@SpringBootApplication",
      scope: "singleton",
      isPrimary: false,
      isLazy: false,
      profiles: [],
      qualifiers: [],
      file: "src/main/java/com/example/DemoApplication.java",
      line: 5,
      dependencies: [],
    },
    {
      id: "OrderCtrl#10",
      name: "orderController",
      className: "OrderController",
      classFqn: "com.example.web.OrderController",
      package: "com.example.web",
      stereotype: "restController",
      stereotypeLabel: "@RestController",
      scope: "singleton",
      isPrimary: false,
      isLazy: false,
      profiles: [],
      qualifiers: [],
      file: "src/main/java/com/example/web/OrderController.java",
      line: 10,
      dependencies: [
        {
          field: "orderService",
          targetType: "OrderService",
          kind: "constructor",
          status: "resolved",
          qualifier: null,
          resolvedIds: ["OrderSvc#12"],
        },
      ],
    },
    {
      id: "OrderSvc#12",
      name: "orderService",
      className: "OrderService",
      classFqn: "com.example.service.OrderService",
      package: "com.example.service",
      stereotype: "service",
      stereotypeLabel: "@Service",
      scope: "singleton",
      isPrimary: false,
      isLazy: false,
      profiles: ["dev"],
      qualifiers: [],
      file: "src/main/java/com/example/service/OrderService.java",
      line: 12,
      dependencies: [
        {
          field: "orderRepo",
          targetType: "OrderRepository",
          kind: "constructor",
          status: "resolved",
          qualifier: null,
          resolvedIds: ["OrderRepo#8"],
        },
      ],
    },
    {
      id: "OrderRepo#8",
      name: "orderRepository",
      className: "OrderRepository",
      classFqn: "com.example.repo.OrderRepository",
      package: "com.example.repo",
      stereotype: "repository",
      stereotypeLabel: "@Repository",
      scope: "singleton",
      isPrimary: true,
      isLazy: false,
      profiles: [],
      qualifiers: [],
      file: "src/main/java/com/example/repo/OrderRepository.java",
      line: 8,
      dependencies: [],
    },
  ],
  beanEdges: [
    { fromId: "OrderCtrl#10", toId: "OrderSvc#12", label: "orderService", isCycle: false },
    { fromId: "OrderSvc#12", toId: "OrderRepo#8", label: "orderRepo", isCycle: false },
  ],
  cycles: [],
  endpoints: [
    {
      id: "ep:0",
      methods: ["GET"],
      rawPath: "/{id}",
      fullPath: "/api/orders/{id}",
      resolvedUrl: "http://localhost:8080/api/orders/{id}",
      kind: "mvc",
      framework: "Spring MVC",
      controller: "OrderController",
      handler: "getOrder",
      serviceName: null,
      queryParams: [],
      hasRequestBody: false,
      duplicateConflict: false,
      runtimeMatched: false,
      file: "src/main/java/com/example/web/OrderController.java",
      line: 18,
    },
    {
      id: "ep:1",
      methods: ["GET"],
      rawPath: "/stream/events",
      fullPath: "/stream/events",
      resolvedUrl: "http://localhost:8080/stream/events",
      kind: "webFlux",
      framework: "Spring WebFlux",
      controller: "RouterConfig",
      handler: "routeEvents",
      serviceName: null,
      queryParams: [],
      hasRequestBody: false,
      duplicateConflict: false,
      runtimeMatched: false,
      file: "src/main/java/com/example/web/RouterConfig.java",
      line: 14,
    },
  ],
  configFiles: [
    { path: "src/main/resources/application.yml", profile: null, format: "yaml", entryCount: 4 },
    { path: "src/main/resources/application-dev.yml", profile: "dev", format: "yaml", entryCount: 2 },
  ],
  effectiveProperties: [
    {
      key: "server.port",
      relaxedKey: "server.port",
      maskedValue: "8080",
      sourceFile: "src/main/resources/application-dev.yml",
      sourceProfile: "dev",
      line: 1,
      overriddenFrom: ["default (application.yml: 8081)"],
    },
    {
      key: "spring.datasource.password",
      relaxedKey: "spring.datasource.password",
      maskedValue: "******",
      sourceFile: "src/main/resources/application-dev.yml",
      sourceProfile: "dev",
      line: 4,
      overriddenFrom: [],
    },
  ],
  configPropClasses: [],
  entities: [
    {
      name: "Order",
      fqn: "com.example.domain.Order",
      tableName: "orders",
      file: "src/main/java/com/example/domain/Order.java",
      line: 6,
      fields: [
        { name: "id", typeName: "Long", columnName: "id", isId: true },
        { name: "status", typeName: "String", columnName: "status", isId: false },
      ],
    },
  ],
  repositories: [
    {
      name: "OrderRepository",
      baseInterface: "JpaRepository",
      entityType: "Order",
      idType: "Long",
      file: "src/main/java/com/example/repo/OrderRepository.java",
      line: 8,
      methods: [
        {
          name: "findByStatus",
          returnType: "List<Order>",
          line: 9,
          query: null,
          jpqlPreview: "SELECT o FROM Order o WHERE o.status = ?1",
          valid: true,
          errors: [],
        },
      ],
    },
  ],
  scheduledSites: [
    {
      methodName: "cleanupExpired",
      cron: "0 */15 * * * *",
      fixedRate: null,
      fixedDelay: null,
      zone: null,
      summary: "Every 15 minutes",
      valid: true,
      error: null,
      file: "src/main/java/com/example/service/OrderService.java",
      line: 25,
    },
  ],
  spelSites: [],
  runtimeInstances: [],
};
