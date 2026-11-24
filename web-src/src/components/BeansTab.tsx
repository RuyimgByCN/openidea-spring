import { useMemo, useState } from "react";
import { bridge, layoutBeanGraph } from "../bridge";
import type { Strings } from "../i18n";
import type { BeanItem, ProjectSnapshot } from "../types";

interface Props {
  snapshot: ProjectSnapshot;
  t: Strings;
  focusedBeanId: string | null;
  onSelectBean: (id: string | null) => void;
}

function stereotypeColor(stereotype: string): string {
  if (stereotype === "service") return "#6db33f";
  if (stereotype === "repository") return "#548af7";
  if (stereotype === "restController" || stereotype === "controller") return "#e0a84b";
  if (stereotype === "configuration" || stereotype === "beanMethod") return "#b476f0";
  if (stereotype === "springBootApplication") return "#35c69b";
  return "#868a91";
}

export function BeansTab({ snapshot, t, focusedBeanId, onSelectBean }: Props) {
  const [query, setQuery] = useState("");
  const [stereotypeFilter, setStereotypeFilter] = useState("ALL");
  const [mode, setMode] = useState<"split" | "graph" | "table">("split");
  const [zoom, setZoom] = useState(1);
  const [pan, setPan] = useState({ x: 0, y: 0 });
  const [dragOrigin, setDragOrigin] = useState<{ x: number; y: number } | null>(null);

  const stereotypes = useMemo(() => {
    const set = new Set<string>();
    for (const b of snapshot.beans) set.add(b.stereotypeLabel);
    return Array.from(set).sort();
  }, [snapshot.beans]);

  const filteredBeans = useMemo(() => {
    const q = query.trim().toLowerCase();
    return snapshot.beans.filter((b) => {
      if (stereotypeFilter !== "ALL" && b.stereotypeLabel !== stereotypeFilter) return false;
      if (!q) return true;
      const hay = `${b.name} ${b.className} ${b.classFqn} ${b.stereotypeLabel}`.toLowerCase();
      return hay.includes(q);
    });
  }, [snapshot.beans, query, stereotypeFilter]);

  const layout = useMemo(
    () => layoutBeanGraph(filteredBeans, snapshot.beanEdges, snapshot.cycles),
    [filteredBeans, snapshot.beanEdges, snapshot.cycles],
  );

  const nodePosById = useMemo(() => {
    const map = new Map<string, (typeof layout.nodes)[number]>();
    for (const n of layout.nodes) map.set(n.bean.id, n);
    return map;
  }, [layout.nodes]);

  const selectedBean: BeanItem | undefined = useMemo(
    () =>
      snapshot.beans.find((b) => b.id === focusedBeanId) ??
      filteredBeans[0],
    [snapshot.beans, filteredBeans, focusedBeanId],
  );

  const incomingForSelected = useMemo(() => {
    if (!selectedBean) return [];
    return snapshot.beanEdges
      .filter((e) => e.toId === selectedBean.id)
      .map((e) => ({
        edge: e,
        from: snapshot.beans.find((b) => b.id === e.fromId),
      }));
  }, [selectedBean, snapshot.beanEdges, snapshot.beans]);

  const openLocation = (file: string, line: number) => {
    void bridge.rpc("editor.open", { path: file, line, character: 0 }).catch(() => {});
  };

  return (
    <div className="sp-split">
      {snapshot.cycles.length > 0 && (
        <div className="sp-banner danger">
          <strong>{t.beans.cycleWarning} ({snapshot.cycles.length}):</strong>{" "}
          {snapshot.cycles.map((c, idx) => (
            <span key={idx} className="sp-mono">
              {idx > 0 ? " | " : ""}
              {c
                .map((id) => snapshot.beans.find((b) => b.id === id)?.className ?? id)
                .join(" → ")}
            </span>
          ))}
        </div>
      )}

      <div className="sp-toolbar">
        <input
          className="sp-input grow"
          placeholder={t.beans.searchPlaceholder}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        <select
          className="sp-select"
          value={stereotypeFilter}
          onChange={(e) => setStereotypeFilter(e.target.value)}
        >
          <option value="ALL">{t.beans.allStereotypes}</option>
          {stereotypes.map((s) => (
            <option key={s} value={s}>
              {s}
            </option>
          ))}
        </select>
        <div className="sp-chips">
          <button
            type="button"
            className={`sp-btn ${mode === "split" ? "active" : ""}`}
            onClick={() => setMode("split")}
          >
            {t.beans.viewSplit}
          </button>
          <button
            type="button"
            className={`sp-btn ${mode === "graph" ? "active" : ""}`}
            onClick={() => setMode("graph")}
          >
            {t.beans.viewGraph}
          </button>
          <button
            type="button"
            className={`sp-btn ${mode === "table" ? "active" : ""}`}
            onClick={() => setMode("table")}
          >
            {t.beans.viewTable}
          </button>
        </div>
      </div>

      {filteredBeans.length === 0 ? (
        <div className="sp-card sp-empty">{t.beans.noBeans}</div>
      ) : (
        <>
          {(mode === "split" || mode === "graph") && (
            <div
              className="sp-graph-wrap"
              style={{ height: mode === "graph" ? 420 : 270 }}
              onMouseDown={(e) => setDragOrigin({ x: e.clientX - pan.x, y: e.clientY - pan.y })}
              onMouseMove={(e) => {
                if (dragOrigin) {
                  setPan({ x: e.clientX - dragOrigin.x, y: e.clientY - dragOrigin.y });
                }
              }}
              onMouseUp={() => setDragOrigin(null)}
              onMouseLeave={() => setDragOrigin(null)}
            >
              <div className="sp-graph-controls">
                <button
                  type="button"
                  className="sp-btn"
                  onClick={() => setZoom((z) => Math.min(2.2, +(z + 0.15).toFixed(2)))}
                >
                  {t.beans.zoomIn}
                </button>
                <button
                  type="button"
                  className="sp-btn"
                  onClick={() => setZoom((z) => Math.max(0.45, +(z - 0.15).toFixed(2)))}
                >
                  {t.beans.zoomOut}
                </button>
                <button
                  type="button"
                  className="sp-btn"
                  onClick={() => {
                    setZoom(1);
                    setPan({ x: 0, y: 0 });
                  }}
                >
                  {t.beans.zoomReset}
                </button>
              </div>
              <svg width="100%" height="100%" style={{ cursor: dragOrigin ? "grabbing" : "grab" }}>
                <defs>
                  <marker
                    id="sp-arrow"
                    viewBox="0 0 10 10"
                    refX="8"
                    refY="5"
                    markerWidth="6"
                    markerHeight="6"
                    orient="auto-start-reverse"
                  >
                    <path d="M 0 1 L 10 5 L 0 9 z" fill="#868a91" />
                  </marker>
                  <marker
                    id="sp-arrow-active"
                    viewBox="0 0 10 10"
                    refX="8"
                    refY="5"
                    markerWidth="6"
                    markerHeight="6"
                    orient="auto-start-reverse"
                  >
                    <path d="M 0 1 L 10 5 L 0 9 z" fill="#6db33f" />
                  </marker>
                  <marker
                    id="sp-arrow-cycle"
                    viewBox="0 0 10 10"
                    refX="8"
                    refY="5"
                    markerWidth="7"
                    markerHeight="7"
                    orient="auto-start-reverse"
                  >
                    <path d="M 0 1 L 10 5 L 0 9 z" fill="#f75464" />
                  </marker>
                </defs>

                <g transform={`translate(${pan.x}, ${pan.y}) scale(${zoom})`}>
                  {layout.edges.map((e, i) => {
                    const from = nodePosById.get(e.fromId);
                    const to = nodePosById.get(e.toId);
                    if (!from || !to) return null;
                    const x1 = from.x + from.width;
                    const y1 = from.y + from.height / 2;
                    const x2 = to.x;
                    const y2 = to.y + to.height / 2;
                    const dx = Math.max(40, Math.abs(x2 - x1) * 0.45);
                    const pathD =
                      x1 <= x2
                        ? `M ${x1} ${y1} C ${x1 + dx} ${y1}, ${x2 - dx} ${y2}, ${x2} ${y2}`
                        : `M ${from.x + from.width / 2} ${from.y} C ${from.x + from.width / 2} ${from.y - 45}, ${to.x + to.width / 2} ${to.y - 45}, ${to.x + to.width / 2} ${to.y}`;
                    const touchesSelected =
                      selectedBean &&
                      (e.fromId === selectedBean.id || e.toId === selectedBean.id);
                    const stroke = e.isCycle
                      ? "#f75464"
                      : touchesSelected
                        ? "#6db33f"
                        : "#555961";
                    const marker = e.isCycle
                      ? "url(#sp-arrow-cycle)"
                      : touchesSelected
                        ? "url(#sp-arrow-active)"
                        : "url(#sp-arrow)";
                    return (
                      <path
                        key={`${e.fromId}-${e.toId}-${i}`}
                        d={pathD}
                        fill="none"
                        stroke={stroke}
                        strokeWidth={e.isCycle ? 2.2 : touchesSelected ? 2 : 1.3}
                        strokeDasharray={e.isCycle ? "5 3" : undefined}
                        opacity={selectedBean && !touchesSelected && !e.isCycle ? 0.32 : 0.95}
                        markerEnd={marker}
                      />
                    );
                  })}

                  {layout.nodes.map((node) => {
                    const b = node.bean;
                    const isSel = selectedBean?.id === b.id;
                    const accent = node.inCycle ? "#f75464" : stereotypeColor(b.stereotype);
                    return (
                      <g
                        key={b.id}
                        transform={`translate(${node.x}, ${node.y})`}
                        style={{ cursor: "pointer" }}
                        onClick={(ev) => {
                          ev.stopPropagation();
                          onSelectBean(b.id);
                        }}
                        onDoubleClick={(ev) => {
                          ev.stopPropagation();
                          openLocation(b.file, b.line);
                        }}
                      >
                        <rect
                          width={node.width}
                          height={node.height}
                          rx={6}
                          fill="var(--sp-bg-elevated)"
                          stroke={isSel ? "#6db33f" : accent}
                          strokeWidth={isSel ? 2.2 : 1.3}
                        />
                        <rect x={0} y={0} width={5} height={node.height} rx={2} fill={accent} />
                        <text
                          x={12}
                          y={20}
                          fontSize={11.5}
                          fontWeight={600}
                          fill="var(--sp-fg-strong)"
                        >
                          {b.className.length > 21 ? `${b.className.slice(0, 20)}…` : b.className}
                        </text>
                        <text
                          x={12}
                          y={38}
                          fontSize={10}
                          fill="var(--sp-fg-muted)"
                          fontFamily="var(--sp-font-mono)"
                        >
                          {b.stereotypeLabel}
                          {b.isPrimary ? " ★" : ""}
                        </text>
                      </g>
                    );
                  })}
                </g>
              </svg>
            </div>
          )}

          {(mode === "split" || mode === "table") && (
            <div className="sp-grid-2">
              <div className="sp-table-wrap" style={{ maxHeight: 280 }}>
                <table className="sp-table">
                  <thead>
                    <tr>
                      <th>Bean</th>
                      <th>Stereotype</th>
                      <th>Deps</th>
                    </tr>
                  </thead>
                  <tbody>
                    {filteredBeans.map((b) => (
                      <tr
                        key={b.id}
                        className={`clickable ${selectedBean?.id === b.id ? "selected" : ""}`}
                        onClick={() => onSelectBean(b.id)}
                        onDoubleClick={() => openLocation(b.file, b.line)}
                      >
                        <td>
                          <div style={{ fontWeight: 600, color: "var(--sp-fg-strong)" }}>
                            {b.className}
                          </div>
                          <div className="sp-mono sp-muted">{b.name}</div>
                        </td>
                        <td>
                          <span className="sp-chip">{b.stereotypeLabel}</span>
                          {b.isPrimary && <span className="sp-chip green">Primary</span>}
                        </td>
                        <td className="sp-mono">{b.dependencies.length}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>

              {selectedBean && (
                <div className="sp-card">
                  <div className="sp-card-title">
                    <span>
                      {selectedBean.className}{" "}
                      <span className="sp-mono sp-muted">({selectedBean.name})</span>
                    </span>
                    <button
                      type="button"
                      className="sp-btn primary"
                      onClick={() => openLocation(selectedBean.file, selectedBean.line)}
                    >
                      {t.beans.openSource}
                    </button>
                  </div>
                  <div className="sp-chips" style={{ marginBottom: 8 }}>
                    <span className="sp-chip green">{selectedBean.stereotypeLabel}</span>
                    <span className="sp-chip">scope: {selectedBean.scope}</span>
                    {selectedBean.isPrimary && <span className="sp-chip blue">@Primary</span>}
                    {selectedBean.profiles.map((p) => (
                      <span key={p} className="sp-chip warn">
                        @Profile({p})
                      </span>
                    ))}
                    {selectedBean.qualifiers.map((q) => (
                      <span key={q} className="sp-chip">
                        @Qualifier({q})
                      </span>
                    ))}
                  </div>
                  <div className="sp-muted" style={{ fontWeight: 600, marginBottom: 4 }}>
                    {t.beans.dependencies} ({selectedBean.dependencies.length})
                  </div>
                  {selectedBean.dependencies.length === 0 ? (
                    <div className="sp-muted" style={{ marginBottom: 8 }}>
                      —
                    </div>
                  ) : (
                    <div className="sp-chips" style={{ marginBottom: 8 }}>
                      {selectedBean.dependencies.map((d, i) => (
                        <button
                          key={`${d.field}-${i}`}
                          type="button"
                          className="sp-btn"
                          onClick={() => {
                            if (d.resolvedIds[0]) onSelectBean(d.resolvedIds[0]);
                          }}
                        >
                          <span className="sp-mono">{d.field}</span>:{" "}
                          <span className="sp-mono">{d.targetType}</span>
                          <span
                            className={`sp-chip ${
                              d.status === "resolved"
                                ? "green"
                                : d.status === "ambiguous"
                                  ? "warn"
                                  : "danger"
                            }`}
                          >
                            {d.status}
                          </span>
                        </button>
                      ))}
                    </div>
                  )}

                  <div className="sp-muted" style={{ fontWeight: 600, marginBottom: 4 }}>
                    {t.beans.dependents} ({incomingForSelected.length})
                  </div>
                  {incomingForSelected.length === 0 ? (
                    <div className="sp-muted">—</div>
                  ) : (
                    <div className="sp-chips">
                      {incomingForSelected.map(({ edge, from }, i) => (
                        <button
                          key={`${edge.fromId}-${i}`}
                          type="button"
                          className="sp-btn"
                          onClick={() => onSelectBean(edge.fromId)}
                        >
                          {from?.className ?? edge.fromId}
                        </button>
                      ))}
                    </div>
                  )}
                </div>
              )}
            </div>
          )}
        </>
      )}
    </div>
  );
}
