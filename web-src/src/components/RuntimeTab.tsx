import { useEffect, useMemo, useState } from "react";
import { bridge } from "../bridge";
import type { Strings } from "../i18n";
import type { ProjectSnapshot, RuntimeAppInstance } from "../types";

interface Props {
  snapshot: ProjectSnapshot;
  t: Strings;
  onRuntimeUpdate: (instances: RuntimeAppInstance[]) => void;
  onConsoleToggle: (enabled: boolean) => void;
  onToast: (msg: string) => void;
}

const LOG_LEVELS = ["TRACE", "DEBUG", "INFO", "WARN", "ERROR", "OFF"] as const;

type SubTab = "health" | "beans" | "mappings" | "env" | "conditions" | "loggers";

interface LoggerRow {
  name: string;
  configuredLevel?: string | null;
  effectiveLevel?: string | null;
}

function parseLoggers(raw: string | null): { rows: LoggerRow[]; note?: string } {
  if (!raw) return { rows: [] };
  try {
    const parsed = JSON.parse(raw) as Record<string, unknown>;
    if (parsed.loggers && typeof parsed.loggers === "object") {
      const entries = Object.entries(parsed.loggers as Record<string, Record<string, string | null>>);
      return {
        rows: entries.slice(0, 120).map(([name, val]) => ({
          name,
          configuredLevel: val?.configuredLevel ?? null,
          effectiveLevel: val?.effectiveLevel ?? null,
        })),
      };
    }
    if (typeof parsed.note === "string") {
      return { rows: [], note: parsed.note };
    }
    return { rows: [] };
  } catch {
    return { rows: [] };
  }
}

function formatJson(raw: string | null): string {
  if (!raw) return "—";
  try {
    return JSON.stringify(JSON.parse(raw), null, 2);
  } catch {
    return raw;
  }
}

export function RuntimeTab({ snapshot, t, onRuntimeUpdate, onConsoleToggle, onToast }: Props) {
  const [actuatorUrl, setActuatorUrl] = useState("http://localhost:8080/actuator");
  const [selectedId, setSelectedId] = useState<string | null>(
    snapshot.runtimeInstances[0]?.id ?? null,
  );
  const [subTab, setSubTab] = useState<SubTab>("health");
  const [filterText, setFilterText] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!selectedId && snapshot.runtimeInstances.length > 0) {
      setSelectedId(snapshot.runtimeInstances[0].id);
    }
  }, [snapshot.runtimeInstances, selectedId]);

  const activeInst = useMemo(
    () =>
      snapshot.runtimeInstances.find((i) => i.id === selectedId) ??
      snapshot.runtimeInstances[0],
    [snapshot.runtimeInstances, selectedId],
  );

  const loggersInfo = useMemo(() => parseLoggers(activeInst?.loggersJson ?? null), [activeInst]);

  const refreshRuntime = async () => {
    setBusy(true);
    try {
      const res = await bridge.rpc<{ instances: RuntimeAppInstance[] }>("runtime.refresh", {});
      onRuntimeUpdate(res.instances);
    } catch (err) {
      onToast(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const connectActuator = async () => {
    setBusy(true);
    try {
      const res = await bridge.rpc<{ instances: RuntimeAppInstance[] }>("runtime.connect", {
        url: actuatorUrl,
      });
      onRuntimeUpdate(res.instances);
    } catch (err) {
      onToast(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const syncDatasource = async () => {
    try {
      const res = await bridge.rpc<{ name: string }>("datasource.sync", {});
      onToast(`Synced: ${res.name}`);
    } catch (err) {
      onToast(err instanceof Error ? err.message : String(err));
    }
  };

  const toggleConsole = async () => {
    try {
      const res = await bridge.rpc<{ enabled: boolean }>("console.toggle", {});
      onConsoleToggle(res.enabled);
    } catch (err) {
      onToast(err instanceof Error ? err.message : String(err));
    }
  };

  const setLogger = async (logger: string, level: string) => {
    if (!activeInst) return;
    try {
      const res = await bridge.rpc<{ instances: RuntimeAppInstance[] }>("runtime.setLogger", {
        instanceId: activeInst.id,
        logger,
        level,
      });
      onRuntimeUpdate(res.instances);
      onToast(`${logger} -> ${level}`);
    } catch (err) {
      onToast(err instanceof Error ? err.message : String(err));
    }
  };

  const rawBody = useMemo(() => {
    if (!activeInst) return "";
    const text =
      subTab === "health"
        ? formatJson(activeInst.healthJson)
        : subTab === "beans"
          ? formatJson(activeInst.liveBeansJson)
          : subTab === "mappings"
            ? formatJson(activeInst.mappingsJson)
            : subTab === "env"
              ? formatJson(activeInst.envJson)
              : formatJson(activeInst.conditionsJson);
    const f = filterText.trim().toLowerCase();
    if (!f) return text;
    return text
      .split("\n")
      .filter((line) => line.toLowerCase().includes(f))
      .join("\n");
  }, [activeInst, subTab, filterText]);

  return (
    <div className="sp-split">
      <div className="sp-toolbar">
        <button
          type="button"
          className="sp-btn primary"
          disabled={busy}
          onClick={() => void refreshRuntime()}
        >
          {t.runtime.refreshRuntime}
        </button>
        <input
          className="sp-input grow sp-mono"
          placeholder={t.runtime.actuatorPlaceholder}
          value={actuatorUrl}
          onChange={(e) => setActuatorUrl(e.target.value)}
        />
        <button
          type="button"
          className="sp-btn"
          disabled={busy}
          onClick={() => void connectActuator()}
        >
          {t.runtime.connectBtn}
        </button>
        <button type="button" className="sp-btn" onClick={() => void syncDatasource()}>
          {t.runtime.syncDatasource}
        </button>
        <button
          type="button"
          className={`sp-btn ${snapshot.consoleHighlighting ? "active" : ""}`}
          onClick={() => void toggleConsole()}
        >
          {t.runtime.consoleToggle}:{" "}
          {snapshot.consoleHighlighting ? t.runtime.consoleOn : t.runtime.consoleOff}
        </button>
      </div>

      {!activeInst ? (
        <div className="sp-card sp-empty">
          <div style={{ fontWeight: 600, marginBottom: 4 }}>{t.runtime.noInstances}</div>
          <div style={{ fontSize: 11 }}>{t.runtime.noInstancesHint}</div>
        </div>
      ) : (
        <>
          <div className="sp-chips">
            {snapshot.runtimeInstances.map((inst) => (
              <button
                key={inst.id}
                type="button"
                className={`sp-btn ${inst.id === activeInst.id ? "active" : ""}`}
                onClick={() => setSelectedId(inst.id)}
              >
                <span className={`sp-chip ${inst.healthStatus === "UP" ? "green" : "warn"}`}>
                  {inst.healthStatus}
                </span>
                <strong>{inst.appName}</strong>
                {inst.serverPort && <span className="sp-mono">:{inst.serverPort}</span>}
                <span className="sp-chip blue">{inst.source}</span>
              </button>
            ))}
            <button
              type="button"
              className="sp-btn"
              onClick={() =>
                void bridge
                  .rpc("runtime.openBrowser", { instanceId: activeInst.id })
                  .catch(() => {})
              }
            >
              {t.runtime.openBrowser}
            </button>
          </div>

          <div className="sp-card">
            <div className="sp-toolbar" style={{ marginBottom: 8 }}>
              <div className="sp-chips">
                {(["health", "beans", "mappings", "env", "conditions", "loggers"] as SubTab[]).map(
                  (st) => (
                    <button
                      key={st}
                      type="button"
                      className={`sp-btn ${subTab === st ? "active" : ""}`}
                      onClick={() => setSubTab(st)}
                    >
                      {t.runtime.subtabs[st]}
                    </button>
                  ),
                )}
              </div>
              <input
                className="sp-input grow"
                placeholder={t.runtime.filterPlaceholder}
                value={filterText}
                onChange={(e) => setFilterText(e.target.value)}
              />
            </div>

            {subTab === "loggers" ? (
              loggersInfo.rows.length === 0 ? (
                <div className="sp-muted">{loggersInfo.note ?? "No logger entries exposed."}</div>
              ) : (
                <div className="sp-table-wrap" style={{ maxHeight: 280 }}>
                  <table className="sp-table">
                    <thead>
                      <tr>
                        <th>{t.runtime.loggerName}</th>
                        <th>{t.runtime.effectiveLevel}</th>
                      </tr>
                    </thead>
                    <tbody>
                      {loggersInfo.rows
                        .filter(
                          (r) =>
                            !filterText.trim() ||
                            r.name.toLowerCase().includes(filterText.trim().toLowerCase()),
                        )
                        .map((row) => (
                          <tr key={row.name}>
                            <td className="sp-mono">{row.name}</td>
                            <td>
                              <div className="sp-chips">
                                {LOG_LEVELS.map((lvl) => {
                                  const active =
                                    (row.configuredLevel ?? row.effectiveLevel) === lvl;
                                  return (
                                    <button
                                      key={lvl}
                                      type="button"
                                      className={`sp-btn ${active ? "active" : ""}`}
                                      onClick={() => void setLogger(row.name, lvl)}
                                    >
                                      {lvl}
                                    </button>
                                  );
                                })}
                              </div>
                            </td>
                          </tr>
                        ))}
                    </tbody>
                  </table>
                </div>
              )
            ) : (
              <pre className="sp-pre">{rawBody}</pre>
            )}
          </div>
        </>
      )}
    </div>
  );
}
