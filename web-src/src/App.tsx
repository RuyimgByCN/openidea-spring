import { useEffect, useState } from "react";
import { applyHostTheme, bridge } from "./bridge";
import { BeansTab } from "./components/BeansTab";
import { ConfigTab } from "./components/ConfigTab";
import { DataSpelTab } from "./components/DataSpelTab";
import { EndpointsTab } from "./components/EndpointsTab";
import { RuntimeTab } from "./components/RuntimeTab";
import { getStrings, isZhLocale } from "./i18n";
import { DEMO_SNAPSHOT, type ProjectSnapshot, type TabId } from "./types";

function parseTabId(raw: unknown): TabId | null {
  if (
    raw === "beans" ||
    raw === "endpoints" ||
    raw === "config" ||
    raw === "data" ||
    raw === "runtime"
  ) {
    return raw;
  }
  return null;
}

export default function App() {
  const [snapshot, setSnapshot] = useState<ProjectSnapshot>(DEMO_SNAPSHOT);
  const [locale, setLocale] = useState("en");
  const [activeTab, setActiveTab] = useState<TabId>("beans");
  const [focusedBeanId, setFocusedBeanId] = useState<string | null>(null);
  const [initialCron, setInitialCron] = useState<string | null>(null);
  const [toast, setToast] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);

  const showToast = (msg: string) => {
    setToast(msg);
    setTimeout(() => {
      setToast((cur) => (cur === msg ? null : cur));
    }, 2800);
  };

  useEffect(() => {
    applyHostTheme(window.openidea?.theme ?? null);
    window.openidea?.onThemeChange?.((theme) => applyHostTheme(theme));

    const off = bridge.onPush((msg) => {
      if (msg.op === "snapshot" && msg.snapshot) {
        const snap = msg.snapshot as ProjectSnapshot;
        setSnapshot(snap);
        if (snap.locale) setLocale(snap.locale);
      } else if (msg.op === "open-tab") {
        const nextTab = parseTabId(msg.tab);
        if (nextTab) setActiveTab(nextTab);
        if (msg.snapshot) {
          const snap = msg.snapshot as ProjectSnapshot;
          setSnapshot(snap);
          if (snap.locale) setLocale(snap.locale);
        }
        const extra = msg.extra as Record<string, unknown> | undefined;
        if (typeof extra?.beanId === "string") setFocusedBeanId(extra.beanId);
        if (typeof extra?.cron === "string") setInitialCron(extra.cron);
      }
    });

    void bridge.requestInit().then((init) => {
      if (init.locale) setLocale(init.locale);
      if (init.snapshot) setSnapshot(init.snapshot);
      const pendingTab = parseTabId(init.pending?.tab);
      if (pendingTab) setActiveTab(pendingTab);
      const extra = init.pending?.extra;
      if (typeof extra?.beanId === "string") setFocusedBeanId(extra.beanId);
      if (typeof extra?.cron === "string") setInitialCron(extra.cron);
    });

    return () => off();
  }, []);

  const t = getStrings(locale);
  const zh = isZhLocale(locale);
  const projectLabel = snapshot.projectRoot.split(/[\\/]/).filter(Boolean).pop() || snapshot.projectRoot;

  const refreshIndex = async () => {
    setRefreshing(true);
    try {
      const next = await bridge.rpc<ProjectSnapshot>("project.refresh", {});
      setSnapshot(next);
    } catch (err) {
      showToast(err instanceof Error ? err.message : String(err));
    } finally {
      setRefreshing(false);
    }
  };

  return (
    <div className="sp-shell">
      <header className="sp-header">
        <div className="sp-title-group">
          <span className="sp-leaf">S</span>
          <span className="sp-title">{t.appTitle}</span>
          {projectLabel && <span className="sp-project-pill">{projectLabel}</span>}
        </div>
        <div className="sp-header-actions">
          <span className="sp-chip green">
            {t.activeProfile}: {snapshot.activeProfile || "default"}
          </span>
          <button
            type="button"
            className="sp-btn"
            disabled={refreshing}
            onClick={() => void refreshIndex()}
          >
            {t.refreshIndex}
          </button>
        </div>
      </header>

      <nav className="sp-tabs">
        <button
          type="button"
          className={`sp-tab ${activeTab === "beans" ? "active" : ""}`}
          onClick={() => setActiveTab("beans")}
        >
          {t.tabs.beans} <span className="sp-count">{snapshot.beans.length}</span>
        </button>
        <button
          type="button"
          className={`sp-tab ${activeTab === "endpoints" ? "active" : ""}`}
          onClick={() => setActiveTab("endpoints")}
        >
          {t.tabs.endpoints} <span className="sp-count">{snapshot.endpoints.length}</span>
        </button>
        <button
          type="button"
          className={`sp-tab ${activeTab === "config" ? "active" : ""}`}
          onClick={() => setActiveTab("config")}
        >
          {t.tabs.config} <span className="sp-count">{snapshot.effectiveProperties.length}</span>
        </button>
        <button
          type="button"
          className={`sp-tab ${activeTab === "data" ? "active" : ""}`}
          onClick={() => setActiveTab("data")}
        >
          {t.tabs.data}
        </button>
        <button
          type="button"
          className={`sp-tab ${activeTab === "runtime" ? "active" : ""}`}
          onClick={() => setActiveTab("runtime")}
        >
          {t.tabs.runtime} <span className="sp-count">{snapshot.runtimeInstances.length}</span>
        </button>
      </nav>

      <main className="sp-main">
        {activeTab === "beans" && (
          <BeansTab
            snapshot={snapshot}
            t={t}
            focusedBeanId={focusedBeanId}
            onSelectBean={setFocusedBeanId}
          />
        )}
        {activeTab === "endpoints" && (
          <EndpointsTab snapshot={snapshot} t={t} onToast={showToast} />
        )}
        {activeTab === "config" && (
          <ConfigTab snapshot={snapshot} t={t} onSnapshot={setSnapshot} onToast={showToast} />
        )}
        {activeTab === "data" && (
          <DataSpelTab snapshot={snapshot} t={t} isZh={zh} initialCron={initialCron} />
        )}
        {activeTab === "runtime" && (
          <RuntimeTab
            snapshot={snapshot}
            t={t}
            onRuntimeUpdate={(instances) =>
              setSnapshot((prev) => ({ ...prev, runtimeInstances: instances }))
            }
            onConsoleToggle={(enabled) =>
              setSnapshot((prev) => ({ ...prev, consoleHighlighting: enabled }))
            }
            onToast={showToast}
          />
        )}
      </main>

      {toast && <div className="sp-toast">{toast}</div>}
    </div>
  );
}
