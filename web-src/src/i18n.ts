export interface Strings {
  appTitle: string;
  refreshIndex: string;
  activeProfile: string;
  noProject: string;
  tabs: {
    beans: string;
    endpoints: string;
    config: string;
    data: string;
    runtime: string;
  };
  beans: {
    searchPlaceholder: string;
    allStereotypes: string;
    viewSplit: string;
    viewGraph: string;
    viewTable: string;
    cycleWarning: string;
    zoomIn: string;
    zoomOut: string;
    zoomReset: string;
    dependencies: string;
    dependents: string;
    noBeans: string;
    openSource: string;
    selectedBean: string;
    status: string;
    qualifiers: string;
    profiles: string;
  };
  endpoints: {
    searchPlaceholder: string;
    allMethods: string;
    allFrameworks: string;
    conflictsOnly: string;
    runtimeOnly: string;
    copyUrl: string;
    copyCurl: string;
    conflictBadge: string;
    liveBadge: string;
    noEndpoints: string;
    copied: string;
  };
  config: {
    starters: string;
    profileSwitcher: string;
    compareProfiles: string;
    leftProfile: string;
    rightProfile: string;
    openDiff: string;
    searchPlaceholder: string;
    key: string;
    value: string;
    source: string;
    overridden: string;
    configProps: string;
    converterTitle: string;
    yamlToProps: string;
    propsToYaml: string;
    convertInputPlaceholder: string;
    noProperties: string;
  };
  data: {
    entitiesTitle: string;
    repositoriesTitle: string;
    spelTitle: string;
    cronTitle: string;
    validateBtn: string;
    explainBtn: string;
    spelPlaceholder: string;
    cronPlaceholder: string;
    valid: string;
    invalid: string;
    fieldsCount: string;
    jpqlPreview: string;
    noEntities: string;
    noRepos: string;
    noScheduled: string;
  };
  runtime: {
    refreshRuntime: string;
    connectActuator: string;
    actuatorPlaceholder: string;
    connectBtn: string;
    syncDatasource: string;
    consoleToggle: string;
    openBrowser: string;
    noInstances: string;
    noInstancesHint: string;
    subtabs: {
      health: string;
      beans: string;
      mappings: string;
      env: string;
      conditions: string;
      loggers: string;
    };
    filterPlaceholder: string;
    loggerName: string;
    effectiveLevel: string;
    consoleOn: string;
    consoleOff: string;
  };
}

const EN: Strings = {
  appTitle: "Spring & Spring Boot",
  refreshIndex: "Refresh Index",
  activeProfile: "Profile",
  noProject: "Open a Spring / Spring Boot project to inspect beans, routes, configuration, data & runtime.",
  tabs: {
    beans: "Beans & Graph",
    endpoints: "Endpoints",
    config: "Config & Profiles",
    data: "Data · SpEL · Cron",
    runtime: "Runtime & Actuator",
  },
  beans: {
    searchPlaceholder: "Search beans by name, class, FQN...",
    allStereotypes: "All Stereotypes",
    viewSplit: "Split",
    viewGraph: "Graph",
    viewTable: "List",
    cycleWarning: "Circular dependency cycle detected",
    zoomIn: "+",
    zoomOut: "−",
    zoomReset: "Reset",
    dependencies: "Injects (Dependencies)",
    dependents: "Injected By (Dependents)",
    noBeans: "No Spring beans indexed.",
    openSource: "Jump to Source",
    selectedBean: "Bean Inspector",
    status: "Wiring",
    qualifiers: "Qualifiers",
    profiles: "Profiles",
  },
  endpoints: {
    searchPlaceholder: "Search path, handler, controller...",
    allMethods: "All",
    allFrameworks: "All Frameworks",
    conflictsOnly: "Conflicts only",
    runtimeOnly: "Runtime live only",
    copyUrl: "Copy URL",
    copyCurl: "Copy cURL",
    conflictBadge: "CONFLICT",
    liveBadge: "LIVE",
    noEndpoints: "No Spring endpoints found.",
    copied: "Copied to clipboard",
  },
  config: {
    starters: "Detected Starters",
    profileSwitcher: "Active Profile",
    compareProfiles: "Compare Profiles (Native Diff)",
    leftProfile: "Left",
    rightProfile: "Right",
    openDiff: "Open Diff",
    searchPlaceholder: "Filter properties by key, value, source...",
    key: "Property Key",
    value: "Effective Value",
    source: "Source",
    overridden: "Overridden",
    configProps: "@ConfigurationProperties",
    converterTitle: "YAML ↔ Properties Converter",
    yamlToProps: "YAML → .properties",
    propsToYaml: ".properties → YAML",
    convertInputPlaceholder: "Paste application.yml or application.properties content...",
    noProperties: "No configuration properties resolved.",
  },
  data: {
    entitiesTitle: "JPA Entities",
    repositoriesTitle: "Spring Data Repositories",
    spelTitle: "SpEL Expression Validator",
    cronTitle: "6-Field Spring Cron Explainer",
    validateBtn: "Validate",
    explainBtn: "Explain",
    spelPlaceholder: "e.g. #{@orderService.computeTotal(#items.?[price > 10])}",
    cronPlaceholder: "e.g. 0 */15 * * * *",
    valid: "Valid",
    invalid: "Invalid",
    fieldsCount: "fields",
    jpqlPreview: "JPQL",
    noEntities: "No @Entity classes found.",
    noRepos: "No Spring Data repositories found.",
    noScheduled: "No @Scheduled tasks found.",
  },
  runtime: {
    refreshRuntime: "Probe Runtime",
    connectActuator: "Connect Actuator",
    actuatorPlaceholder: "http://localhost:8080/actuator",
    connectBtn: "Connect",
    syncDatasource: "Sync Datasource",
    consoleToggle: "Console Highlight",
    openBrowser: "Open Browser",
    noInstances: "No live Spring Boot instances detected.",
    noInstancesHint:
      "Start your Spring Boot app (using the optional Java Probe Agent or Actuator /actuator/*) or connect an Actuator URL above.",
    subtabs: {
      health: "Health",
      beans: "Live Beans",
      mappings: "Mappings",
      env: "Env (Masked)",
      conditions: "Conditions",
      loggers: "Loggers",
    },
    filterPlaceholder: "Filter JSON / loggers...",
    loggerName: "Logger",
    effectiveLevel: "Level",
    consoleOn: "On",
    consoleOff: "Off",
  },
};

const ZH: Strings = {
  appTitle: "Spring & Spring Boot",
  refreshIndex: "刷新索引",
  activeProfile: "Profile",
  noProject: "请先打开一个 Spring / Spring Boot 项目以查看 Bean、端点、配置、数据层和运行时状态。",
  tabs: {
    beans: "Bean 与依赖图",
    endpoints: "API 端点",
    config: "配置与 Profile",
    data: "Data · SpEL · Cron",
    runtime: "运行时与 Actuator",
  },
  beans: {
    searchPlaceholder: "按 Bean 名称、类名或包路径搜索...",
    allStereotypes: "全部注解类型",
    viewSplit: "分栏",
    viewGraph: "依赖图",
    viewTable: "列表",
    cycleWarning: "检测到循环依赖",
    zoomIn: "+",
    zoomOut: "−",
    zoomReset: "重置",
    dependencies: "注入的依赖 (Outgoing)",
    dependents: "被以下 Bean 注入 (Incoming)",
    noBeans: "暂未索引到 Spring Bean。",
    openSource: "跳转到源码",
    selectedBean: "Bean 详情检查器",
    status: "装配状态",
    qualifiers: "Qualifier 限定符",
    profiles: "Profiles",
  },
  endpoints: {
    searchPlaceholder: "搜索路径、处理器方法、Controller...",
    allMethods: "全部",
    allFrameworks: "全部框架",
    conflictsOnly: "仅冲突端点",
    runtimeOnly: "仅运行时在线",
    copyUrl: "复制 URL",
    copyCurl: "复制 cURL",
    conflictBadge: "路由冲突",
    liveBadge: "在线",
    noEndpoints: "未发现 Spring 端点。",
    copied: "已复制到剪贴板",
  },
  config: {
    starters: "已识别 Starters",
    profileSwitcher: "激活 Profile",
    compareProfiles: "对比 Profile（原生 Diff）",
    leftProfile: "左侧",
    rightProfile: "右侧",
    openDiff: "打开 Diff",
    searchPlaceholder: "按配置 Key、值或来源筛选...",
    key: "配置键",
    value: "生效值",
    source: "来源",
    overridden: "覆盖历史",
    configProps: "@ConfigurationProperties",
    converterTitle: "YAML ↔ Properties 转换器",
    yamlToProps: "YAML → .properties",
    propsToYaml: ".properties → YAML",
    convertInputPlaceholder: "粘贴 application.yml 或 application.properties 内容...",
    noProperties: "暂无生效配置项。",
  },
  data: {
    entitiesTitle: "JPA 实体",
    repositoriesTitle: "Spring Data 仓库",
    spelTitle: "SpEL 表达式校验器",
    cronTitle: "6 位 Spring Cron 解析器",
    validateBtn: "校验",
    explainBtn: "解析",
    spelPlaceholder: "例如：#{@orderService.computeTotal(#items.?[price > 10])}",
    cronPlaceholder: "例如：0 */15 * * * *",
    valid: "有效",
    invalid: "无效",
    fieldsCount: "字段",
    jpqlPreview: "JPQL",
    noEntities: "未发现 @Entity 实体类。",
    noRepos: "未发现 Spring Data Repository。",
    noScheduled: "未发现 @Scheduled 定时任务。",
  },
  runtime: {
    refreshRuntime: "探测实例",
    connectActuator: "连接 Actuator",
    actuatorPlaceholder: "http://localhost:8080/actuator",
    connectBtn: "连接",
    syncDatasource: "同步数据源",
    consoleToggle: "控制台高亮",
    openBrowser: "在浏览器打开",
    noInstances: "暂未探测到运行中的 Spring Boot 实例。",
    noInstancesHint:
      "使用可选 Java 运行时探针或 Spring Boot Actuator 启动应用，或在上方输入 Actuator 地址。",
    subtabs: {
      health: "健康检查",
      beans: "实时 Bean",
      mappings: "实时路由",
      env: "环境变量（脱敏）",
      conditions: "条件评估",
      loggers: "日志级别控制",
    },
    filterPlaceholder: "筛选 JSON / 日志包名...",
    loggerName: "Logger 名称",
    effectiveLevel: "级别",
    consoleOn: "开启",
    consoleOff: "关闭",
  },
};

export function getStrings(locale: string): Strings {
  return locale.toLowerCase().startsWith("zh") ? ZH : EN;
}

export function isZhLocale(locale: string): boolean {
  return locale.toLowerCase().startsWith("zh");
}
