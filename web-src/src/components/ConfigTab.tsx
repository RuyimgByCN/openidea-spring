import { useMemo, useState } from "react";
import { bridge } from "../bridge";
import type { Strings } from "../i18n";
import type { ProjectSnapshot } from "../types";

interface Props {
  snapshot: ProjectSnapshot;
  t: Strings;
  onSnapshot: (snap: ProjectSnapshot) => void;
  onToast: (msg: string) => void;
}

export function ConfigTab({ snapshot, t, onSnapshot, onToast }: Props) {
  const [query, setQuery] = useState("");
  const [leftProfile, setLeftProfile] = useState("default");
  const [rightProfile, setRightProfile] = useState(
    snapshot.discoveredProfiles.find((p) => p !== "default") ?? "default",
  );
  const [convertInput, setConvertInput] = useState("server:\n  port: 8080\nspring:\n  application:\n    name: demo\n");
  const [convertOutput, setConvertOutput] = useState("");

  const filteredProps = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return snapshot.effectiveProperties;
    return snapshot.effectiveProperties.filter((p) =>
      `${p.key} ${p.maskedValue} ${p.sourceProfile} ${p.sourceFile}`.toLowerCase().includes(q),
    );
  }, [snapshot.effectiveProperties, query]);

  const badges = snapshot.starters.badges ?? [];

  const selectProfile = async (prof: string) => {
    try {
      const next = await bridge.rpc<ProjectSnapshot>("profile.set", {
        profile: prof === "default" ? "" : prof,
      });
      onSnapshot(next);
    } catch (err) {
      onToast(err instanceof Error ? err.message : String(err));
    }
  };

  const openDiff = async () => {
    try {
      await bridge.rpc("profile.diff", { leftProfile, rightProfile });
    } catch (err) {
      onToast(err instanceof Error ? err.message : String(err));
    }
  };

  const runConvert = async (format: "yaml" | "properties") => {
    try {
      const res = await bridge.rpc<{ converted: string }>("config.convert", {
        format,
        text: convertInput,
      });
      setConvertOutput(res.converted);
    } catch (err) {
      onToast(err instanceof Error ? err.message : String(err));
    }
  };

  return (
    <div className="sp-split">
      {badges.length > 0 && (
        <div className="sp-card">
          <div className="sp-card-title">{t.config.starters}</div>
          <div className="sp-chips">
            {badges.map((b) => (
              <span key={b} className="sp-chip green">
                {b}
              </span>
            ))}
          </div>
        </div>
      )}

      <div className="sp-grid-2">
        <div className="sp-card">
          <div className="sp-card-title">{t.config.profileSwitcher}</div>
          <div className="sp-chips">
            {snapshot.discoveredProfiles.map((p) => {
              const isActive = (snapshot.activeProfile ?? "default") === p;
              return (
                <button
                  key={p}
                  type="button"
                  className={`sp-btn ${isActive ? "active" : ""}`}
                  onClick={() => void selectProfile(p)}
                >
                  {p}
                </button>
              );
            })}
          </div>
        </div>

        <div className="sp-card">
          <div className="sp-card-title">{t.config.compareProfiles}</div>
          <div className="sp-toolbar">
            <label className="sp-muted">{t.config.leftProfile}:</label>
            <select
              className="sp-select"
              value={leftProfile}
              onChange={(e) => setLeftProfile(e.target.value)}
            >
              {snapshot.discoveredProfiles.map((p) => (
                <option key={p} value={p}>
                  {p}
                </option>
              ))}
            </select>
            <label className="sp-muted">{t.config.rightProfile}:</label>
            <select
              className="sp-select"
              value={rightProfile}
              onChange={(e) => setRightProfile(e.target.value)}
            >
              {snapshot.discoveredProfiles.map((p) => (
                <option key={p} value={p}>
                  {p}
                </option>
              ))}
            </select>
            <button type="button" className="sp-btn primary" onClick={() => void openDiff()}>
              {t.config.openDiff}
            </button>
          </div>
        </div>
      </div>

      <div className="sp-toolbar">
        <input
          className="sp-input grow"
          placeholder={t.config.searchPlaceholder}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
      </div>

      {filteredProps.length === 0 ? (
        <div className="sp-card sp-empty">{t.config.noProperties}</div>
      ) : (
        <div className="sp-table-wrap" style={{ maxHeight: 260 }}>
          <table className="sp-table">
            <thead>
              <tr>
                <th>{t.config.key}</th>
                <th>{t.config.value}</th>
                <th>{t.config.source}</th>
              </tr>
            </thead>
            <tbody>
              {filteredProps.map((p) => (
                <tr
                  key={`${p.key}-${p.sourceFile}-${p.line}`}
                  className="clickable"
                  onClick={() =>
                    void bridge
                      .rpc("editor.open", { path: p.sourceFile, line: p.line, character: 0 })
                      .catch(() => {})
                  }
                >
                  <td className="sp-mono" style={{ color: "var(--sp-fg-strong)" }}>
                    {p.key}
                  </td>
                  <td className="sp-mono">{p.maskedValue}</td>
                  <td>
                    <span className="sp-chip blue">{p.sourceProfile}</span>
                    {p.overriddenFrom.length > 0 && (
                      <div className="sp-muted" style={{ fontSize: 10.5, marginTop: 2 }}>
                        {t.config.overridden}: {p.overriddenFrom.join(", ")}
                      </div>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {snapshot.configPropClasses.length > 0 && (
        <div className="sp-card">
          <div className="sp-card-title">{t.config.configProps}</div>
          <div className="sp-chips">
            {snapshot.configPropClasses.map((cp) => (
              <button
                key={cp.className}
                type="button"
                className="sp-btn"
                onClick={() =>
                  void bridge
                    .rpc("editor.open", { path: cp.file, line: cp.line, character: 0 })
                    .catch(() => {})
                }
              >
                <span className="sp-mono">{cp.prefix}.*</span> → {cp.className} ({cp.fields.length})
              </button>
            ))}
          </div>
        </div>
      )}

      <div className="sp-card">
        <div className="sp-card-title">
          <span>{t.config.converterTitle}</span>
          <div className="sp-chips">
            <button type="button" className="sp-btn" onClick={() => void runConvert("yaml")}>
              {t.config.yamlToProps}
            </button>
            <button type="button" className="sp-btn" onClick={() => void runConvert("properties")}>
              {t.config.propsToYaml}
            </button>
          </div>
        </div>
        <div className="sp-grid-2">
          <textarea
            className="sp-textarea"
            rows={5}
            placeholder={t.config.convertInputPlaceholder}
            value={convertInput}
            onChange={(e) => setConvertInput(e.target.value)}
          />
          <pre className="sp-pre" style={{ minHeight: 92 }}>
            {convertOutput || "..."}
          </pre>
        </div>
      </div>
    </div>
  );
}
