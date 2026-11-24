import { useEffect, useState } from "react";
import { bridge } from "../bridge";
import type { Strings } from "../i18n";
import type { CronExplainResult, ProjectSnapshot, SpelValidateResult } from "../types";

interface Props {
  snapshot: ProjectSnapshot;
  t: Strings;
  isZh: boolean;
  initialCron?: string | null;
}

export function DataSpelTab({ snapshot, t, isZh, initialCron }: Props) {
  const [spelInput, setSpelInput] = useState("#{@orderService.getClass().getSimpleName()}");
  const [spelResult, setSpelResult] = useState<SpelValidateResult | null>(null);
  const [cronInput, setCronInput] = useState(initialCron || "0 */15 * * * *");
  const [cronResult, setCronResult] = useState<CronExplainResult | null>(null);

  const validateSpel = async (expr: string) => {
    try {
      const res = await bridge.rpc<SpelValidateResult>("spel.validate", { expression: expr });
      setSpelResult(res);
    } catch {
      setSpelResult({
        valid: expr.trim().length > 0,
        summary: expr,
        error: null,
        beanRefs: [],
        propRefs: [],
      });
    }
  };

  const explainCron = async (expr: string) => {
    try {
      const res = await bridge.rpc<CronExplainResult>("cron.explain", { expression: expr });
      setCronResult(res);
    } catch {
      setCronResult(null);
    }
  };

  useEffect(() => {
    if (initialCron) {
      setCronInput(initialCron);
      void explainCron(initialCron);
    }
  }, [initialCron]);

  const jump = (file: string, line: number) => {
    void bridge.rpc("editor.open", { path: file, line, character: 0 }).catch(() => {});
  };

  return (
    <div className="sp-split">
      <div className="sp-grid-2">
        <div className="sp-card">
          <div className="sp-card-title">{t.data.spelTitle}</div>
          <div className="sp-toolbar" style={{ marginBottom: 6 }}>
            <input
              className="sp-input grow sp-mono"
              placeholder={t.data.spelPlaceholder}
              value={spelInput}
              onChange={(e) => setSpelInput(e.target.value)}
            />
            <button
              type="button"
              className="sp-btn primary"
              onClick={() => void validateSpel(spelInput)}
            >
              {t.data.validateBtn}
            </button>
          </div>
          {spelResult && (
            <div className={`sp-banner ${spelResult.valid ? "success" : "danger"}`}>
              <strong>{spelResult.valid ? t.data.valid : t.data.invalid}:</strong>{" "}
              {spelResult.error || spelResult.summary}
              {spelResult.beanRefs.length > 0 && (
                <div className="sp-chips" style={{ marginTop: 4 }}>
                  {spelResult.beanRefs.map((b) => (
                    <span key={b} className="sp-chip green">
                      @{b}
                    </span>
                  ))}
                </div>
              )}
            </div>
          )}
          {snapshot.spelSites.length > 0 && (
            <div className="sp-chips" style={{ marginTop: 8 }}>
              {snapshot.spelSites.map((sp, idx) => (
                <button
                  key={idx}
                  type="button"
                  className="sp-btn sp-mono"
                  onClick={() => {
                    setSpelInput(sp.expr);
                    void validateSpel(sp.expr);
                    jump(sp.file, sp.line);
                  }}
                >
                  {sp.expr}
                </button>
              ))}
            </div>
          )}
        </div>

        <div className="sp-card">
          <div className="sp-card-title">{t.data.cronTitle}</div>
          <div className="sp-toolbar" style={{ marginBottom: 6 }}>
            <input
              className="sp-input grow sp-mono"
              placeholder={t.data.cronPlaceholder}
              value={cronInput}
              onChange={(e) => setCronInput(e.target.value)}
            />
            <button
              type="button"
              className="sp-btn primary"
              onClick={() => void explainCron(cronInput)}
            >
              {t.data.explainBtn}
            </button>
          </div>
          {cronResult && (
            <div className={`sp-banner ${cronResult.valid ? "success" : "danger"}`}>
              <div>
                <strong>{cronResult.valid ? t.data.valid : t.data.invalid}:</strong>{" "}
                {cronResult.error || (isZh ? cronResult.summaryZh : cronResult.summaryEn)}
              </div>
              {cronResult.fields.length > 0 && (
                <div className="sp-chips" style={{ marginTop: 4 }}>
                  {cronResult.fields.map(([k, v]) => (
                    <span key={k} className="sp-chip sp-mono">
                      {k}: {v}
                    </span>
                  ))}
                </div>
              )}
            </div>
          )}
          {snapshot.scheduledSites.length === 0 ? (
            <div className="sp-muted" style={{ marginTop: 6 }}>
              {t.data.noScheduled}
            </div>
          ) : (
            <div className="sp-chips" style={{ marginTop: 8 }}>
              {snapshot.scheduledSites.map((s, idx) => (
                <button
                  key={idx}
                  type="button"
                  className="sp-btn"
                  onClick={() => {
                    if (s.cron) {
                      setCronInput(s.cron);
                      void explainCron(s.cron);
                    }
                    jump(s.file, s.line);
                  }}
                >
                  <span style={{ fontWeight: 600 }}>{s.methodName}</span>
                  <span className="sp-mono sp-muted">
                    {s.cron || s.fixedRate || s.fixedDelay}
                  </span>
                </button>
              ))}
            </div>
          )}
        </div>
      </div>

      <div className="sp-grid-2">
        <div className="sp-card">
          <div className="sp-card-title">
            {t.data.entitiesTitle} ({snapshot.entities.length})
          </div>
          {snapshot.entities.length === 0 ? (
            <div className="sp-muted">{t.data.noEntities}</div>
          ) : (
            <div className="sp-chips">
              {snapshot.entities.map((e) => (
                <button
                  key={e.fqn}
                  type="button"
                  className="sp-btn"
                  onClick={() => jump(e.file, e.line)}
                >
                  <strong>{e.name}</strong>
                  <span className="sp-mono sp-muted">@{e.tableName}</span>
                  <span className="sp-chip">
                    {e.fields.length} {t.data.fieldsCount}
                  </span>
                </button>
              ))}
            </div>
          )}
        </div>

        <div className="sp-card">
          <div className="sp-card-title">
            {t.data.repositoriesTitle} ({snapshot.repositories.length})
          </div>
          {snapshot.repositories.length === 0 ? (
            <div className="sp-muted">{t.data.noRepos}</div>
          ) : (
            <div className="sp-split">
              {snapshot.repositories.map((r) => (
                <div key={r.name} style={{ borderBottom: "1px solid var(--sp-border)", paddingBottom: 6 }}>
                  <div
                    style={{ fontWeight: 600, color: "var(--sp-fg-strong)", cursor: "pointer" }}
                    onClick={() => jump(r.file, r.line)}
                  >
                    {r.name}{" "}
                    <span className="sp-mono sp-muted">
                      &lt;{r.entityType}, {r.idType}&gt;
                    </span>
                  </div>
                  {r.methods.map((m) => (
                    <div
                      key={`${r.name}-${m.name}-${m.line}`}
                      style={{ marginTop: 4, paddingLeft: 8, cursor: "pointer" }}
                      onClick={() => jump(r.file, m.line)}
                    >
                      <span className="sp-mono">{m.name}(…)</span>
                      <span className={`sp-chip ${m.valid ? "green" : "danger"}`} style={{ marginLeft: 6 }}>
                        {m.valid ? t.data.valid : t.data.invalid}
                      </span>
                      {m.jpqlPreview && (
                        <div className="sp-mono sp-muted" style={{ fontSize: 10.5 }}>
                          {t.data.jpqlPreview}: {m.jpqlPreview}
                        </div>
                      )}
                      {m.query && (
                        <div className="sp-mono sp-muted" style={{ fontSize: 10.5 }}>
                          {m.query.native ? "SQL" : "JPQL"}: {m.query.text}
                        </div>
                      )}
                    </div>
                  ))}
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
