import { useMemo, useState } from "react";
import { bridge, filterEndpoints } from "../bridge";
import type { Strings } from "../i18n";
import type { ProjectSnapshot, UnifiedEndpointItem } from "../types";

interface Props {
  snapshot: ProjectSnapshot;
  t: Strings;
  onToast: (msg: string) => void;
}

const HTTP_METHODS = ["ALL", "GET", "POST", "PUT", "DELETE", "PATCH"];

export function EndpointsTab({ snapshot, t, onToast }: Props) {
  const [query, setQuery] = useState("");
  const [method, setMethod] = useState("ALL");
  const [framework, setFramework] = useState("ALL");
  const [conflictsOnly, setConflictsOnly] = useState(false);
  const [runtimeOnly, setRuntimeOnly] = useState(false);

  const items = useMemo(
    () =>
      filterEndpoints(snapshot.endpoints, {
        query,
        method,
        framework,
        conflictsOnly,
        runtimeOnly,
      }),
    [snapshot.endpoints, query, method, framework, conflictsOnly, runtimeOnly],
  );

  const openEndpoint = (ep: UnifiedEndpointItem) => {
    void bridge.rpc("editor.open", { path: ep.file, line: ep.line, character: 0 }).catch(() => {});
  };

  const copyUrl = async (ep: UnifiedEndpointItem) => {
    try {
      await bridge.rpc("clipboard.copy", { text: ep.resolvedUrl });
      onToast(`${t.endpoints.copied}: ${ep.resolvedUrl}`);
    } catch {
      onToast(ep.resolvedUrl);
    }
  };

  const copyCurl = async (ep: UnifiedEndpointItem) => {
    try {
      const res = await bridge.rpc<{ curl: string }>("endpoint.curl", { id: ep.id });
      onToast(`${t.endpoints.copied}: ${res.curl}`);
    } catch {
      const m = ep.methods[0] ?? "GET";
      onToast(`curl -X ${m} "${ep.resolvedUrl}"`);
    }
  };

  return (
    <div className="sp-split">
      <div className="sp-toolbar">
        <input
          className="sp-input grow"
          placeholder={t.endpoints.searchPlaceholder}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        <select className="sp-select" value={framework} onChange={(e) => setFramework(e.target.value)}>
          <option value="ALL">{t.endpoints.allFrameworks}</option>
          <option value="mvc">Spring MVC</option>
          <option value="webFlux">Spring WebFlux</option>
          <option value="httpExchange">@HttpExchange</option>
          <option value="feignClient">@FeignClient</option>
        </select>
      </div>

      <div className="sp-toolbar">
        <div className="sp-chips">
          {HTTP_METHODS.map((m) => (
            <button
              key={m}
              type="button"
              className={`sp-btn ${method === m ? "active" : ""}`}
              onClick={() => setMethod(m)}
            >
              {m === "ALL" ? t.endpoints.allMethods : m}
            </button>
          ))}
        </div>
        <label className="sp-btn" style={{ gap: 5 }}>
          <input
            type="checkbox"
            checked={conflictsOnly}
            onChange={(e) => setConflictsOnly(e.target.checked)}
          />
          {t.endpoints.conflictsOnly}
        </label>
        <label className="sp-btn" style={{ gap: 5 }}>
          <input
            type="checkbox"
            checked={runtimeOnly}
            onChange={(e) => setRuntimeOnly(e.target.checked)}
          />
          {t.endpoints.runtimeOnly}
        </label>
      </div>

      {items.length === 0 ? (
        <div className="sp-card sp-empty">{t.endpoints.noEndpoints}</div>
      ) : (
        <div className="sp-table-wrap">
          <table className="sp-table">
            <thead>
              <tr>
                <th>Method</th>
                <th>Path & Handler</th>
                <th>Framework</th>
                <th>Actions</th>
              </tr>
            </thead>
            <tbody>
              {items.map((ep) => (
                <tr key={ep.id} className="clickable" onClick={() => openEndpoint(ep)}>
                  <td>
                    <div className="sp-chips">
                      {ep.methods.map((m) => (
                        <span key={m} className={`sp-method ${m}`}>
                          {m}
                        </span>
                      ))}
                    </div>
                  </td>
                  <td>
                    <div className="sp-mono" style={{ fontWeight: 600, color: "var(--sp-fg-strong)" }}>
                      {ep.fullPath}
                    </div>
                    <div className="sp-muted" style={{ fontSize: 11 }}>
                      {ep.controller}.{ep.handler}()
                      {ep.duplicateConflict && (
                        <span className="sp-chip danger" style={{ marginLeft: 6 }}>
                          {t.endpoints.conflictBadge}
                        </span>
                      )}
                      {ep.runtimeMatched && (
                        <span className="sp-chip green" style={{ marginLeft: 6 }}>
                          {t.endpoints.liveBadge}
                        </span>
                      )}
                    </div>
                  </td>
                  <td>
                    <span className="sp-chip blue">{ep.framework}</span>
                  </td>
                  <td onClick={(e) => e.stopPropagation()}>
                    <div className="sp-chips">
                      <button type="button" className="sp-btn" onClick={() => void copyUrl(ep)}>
                        {t.endpoints.copyUrl}
                      </button>
                      <button type="button" className="sp-btn" onClick={() => void copyCurl(ep)}>
                        {t.endpoints.copyCurl}
                      </button>
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
