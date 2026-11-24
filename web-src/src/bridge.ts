import {
  DEMO_SNAPSHOT,
  type BeanGraphEdge,
  type BeanItem,
  type InitPayload,
  type UnifiedEndpointItem,
} from "./types";

export interface OpenIdeaTheme {
  dark: boolean;
  vars: Record<string, string>;
}

export interface OpenIdeaApi {
  postMessage: (text: string) => void;
  onMessage: (fn: (text: string) => void) => void;
  theme?: OpenIdeaTheme | null;
  onThemeChange?: (fn: (theme: OpenIdeaTheme) => void) => void;
}

declare global {
  interface Window {
    openidea?: OpenIdeaApi;
  }
}

export type PushListener = (msg: Record<string, unknown>) => void;

export class BridgeClient {
  private nextId = 1;
  private pending = new Map<
    number,
    { resolve: (v: unknown) => void; reject: (e: Error) => void; timer: ReturnType<typeof setTimeout> }
  >();
  private pushListeners = new Set<PushListener>();
  private postFn: (text: string) => void;
  private hasHost: boolean;

  constructor(host?: Pick<OpenIdeaApi, "postMessage" | "onMessage"> | null) {
    if (host) {
      this.hasHost = true;
      this.postFn = (text) => host.postMessage(text);
      host.onMessage((raw) => this.handleIncoming(raw));
    } else {
      this.hasHost = false;
      this.postFn = () => {};
    }
  }

  get isHosted(): boolean {
    return this.hasHost;
  }

  handleIncoming(raw: string): void {
    let msg: Record<string, unknown>;
    try {
      msg = JSON.parse(raw) as Record<string, unknown>;
    } catch {
      return;
    }
    if (!msg || typeof msg !== "object") return;
    if (msg.op === "rpc-result" && typeof msg.id === "number") {
      const entry = this.pending.get(msg.id);
      if (!entry) return;
      this.pending.delete(msg.id);
      clearTimeout(entry.timer);
      if (msg.ok) {
        entry.resolve(msg.data);
      } else {
        entry.reject(new Error(typeof msg.error === "string" ? msg.error : "RPC failed"));
      }
      return;
    }
    for (const fn of this.pushListeners) {
      fn(msg);
    }
  }

  onPush(fn: PushListener): () => void {
    this.pushListeners.add(fn);
    return () => {
      this.pushListeners.delete(fn);
    };
  }

  rpc<T = unknown>(method: string, params: Record<string, unknown> = {}, timeoutMs = 15000): Promise<T> {
    if (!this.hasHost) {
      return Promise.reject(new Error(`Host unavailable in standalone preview (${method})`));
    }
    const id = this.nextId++;
    return new Promise<T>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`RPC timeout: ${method}`));
      }, timeoutMs);
      this.pending.set(id, {
        resolve: resolve as (v: unknown) => void,
        reject,
        timer,
      });
      this.postFn(JSON.stringify({ op: "rpc", id, method, params }));
    });
  }

  requestInit(timeoutMs = 5000): Promise<InitPayload> {
    if (!this.hasHost) {
      return Promise.resolve({
        op: "init",
        locale: "en",
        pending: null,
        snapshot: DEMO_SNAPSHOT,
      });
    }
    return new Promise<InitPayload>((resolve) => {
      let settled = false;
      const off = this.onPush((msg) => {
        if (msg.op === "init" && !settled) {
          settled = true;
          clearTimeout(timer);
          off();
          resolve(msg as unknown as InitPayload);
        }
      });
      const timer = setTimeout(() => {
        if (!settled) {
          settled = true;
          off();
          resolve({
            op: "init",
            locale: "en",
            pending: null,
            snapshot: DEMO_SNAPSHOT,
          });
        }
      }, timeoutMs);
      this.postFn(JSON.stringify({ op: "ready" }));
    });
  }
}

const hostApi = typeof window !== "undefined" ? window.openidea : undefined;
export const bridge = new BridgeClient(hostApi);

export function applyHostTheme(theme?: OpenIdeaTheme | null): void {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  if (!theme) return;
  root.setAttribute("data-theme", theme.dark ? "dark" : "light");
  if (theme.vars) {
    for (const [k, v] of Object.entries(theme.vars)) {
      root.style.setProperty(k.startsWith("--") ? k : `--${k}`, v);
    }
  }
}

export interface GraphNodePos {
  bean: BeanItem;
  x: number;
  y: number;
  width: number;
  height: number;
  layer: number;
  inCycle: boolean;
}

export interface GraphLayout {
  nodes: GraphNodePos[];
  edges: BeanGraphEdge[];
  width: number;
  height: number;
}

export function layoutBeanGraph(
  beans: BeanItem[],
  edges: BeanGraphEdge[],
  cycles: string[][],
): GraphLayout {
  const nodeWidth = 176;
  const nodeHeight = 52;
  const colGap = 240;
  const rowGap = 78;
  const padX = 28;
  const padY = 28;

  if (beans.length === 0) {
    return { nodes: [], edges: [], width: 400, height: 240 };
  }

  const beanIds = new Set(beans.map((b) => b.id));
  const cycleIds = new Set<string>();
  for (const cyc of cycles) {
    for (const id of cyc) cycleIds.add(id);
  }
  const visibleEdges = edges.filter((e) => beanIds.has(e.fromId) && beanIds.has(e.toId));

  // Longest-path layering from root controllers/apps down to repositories
  const inDeg = new Map<string, number>();
  const outgoing = new Map<string, string[]>();
  for (const b of beans) {
    inDeg.set(b.id, 0);
    outgoing.set(b.id, []);
  }
  for (const e of visibleEdges) {
    outgoing.get(e.fromId)?.push(e.toId);
    inDeg.set(e.toId, (inDeg.get(e.toId) ?? 0) + 1);
  }

  const layerMap = new Map<string, number>();
  const queue: string[] = [];
  for (const b of beans) {
    if ((inDeg.get(b.id) ?? 0) === 0) {
      queue.push(b.id);
      layerMap.set(b.id, 0);
    }
  }
  if (queue.length === 0 && beans.length > 0) {
    queue.push(beans[0].id);
    layerMap.set(beans[0].id, 0);
  }

  let qi = 0;
  const visited = new Set<string>();
  while (qi < queue.length) {
    const cur = queue[qi++];
    if (visited.has(cur)) continue;
    visited.add(cur);
    const curLayer = layerMap.get(cur) ?? 0;
    for (const nxt of outgoing.get(cur) ?? []) {
      const nextLayer = curLayer + 1;
      if ((layerMap.get(nxt) ?? -1) < nextLayer && nextLayer < beans.length + 2) {
        layerMap.set(nxt, nextLayer);
        queue.push(nxt);
      }
    }
  }

  for (const b of beans) {
    if (!layerMap.has(b.id)) {
      const fallback = b.stereotype.includes("controller")
        ? 0
        : b.stereotype === "service"
          ? 1
          : b.stereotype === "repository"
            ? 2
            : 1;
      layerMap.set(b.id, fallback);
    }
  }

  const columns = new Map<number, BeanItem[]>();
  let maxLayer = 0;
  for (const b of beans) {
    const l = layerMap.get(b.id) ?? 0;
    if (l > maxLayer) maxLayer = l;
    const arr = columns.get(l) ?? [];
    arr.push(b);
    columns.set(l, arr);
  }

  let maxRows = 1;
  const nodes: GraphNodePos[] = [];
  for (let l = 0; l <= maxLayer; l++) {
    const colBeans = columns.get(l) ?? [];
    if (colBeans.length > maxRows) maxRows = colBeans.length;
    colBeans.forEach((bean, idx) => {
      nodes.push({
        bean,
        x: padX + l * colGap,
        y: padY + idx * rowGap,
        width: nodeWidth,
        height: nodeHeight,
        layer: l,
        inCycle: cycleIds.has(bean.id),
      });
    });
  }

  return {
    nodes,
    edges: visibleEdges,
    width: Math.max(520, padX * 2 + (maxLayer + 1) * colGap - (colGap - nodeWidth)),
    height: Math.max(260, padY * 2 + maxRows * rowGap),
  };
}

export function filterEndpoints(
  items: UnifiedEndpointItem[],
  opts: {
    query: string;
    method: string;
    framework: string;
    conflictsOnly: boolean;
    runtimeOnly: boolean;
  },
): UnifiedEndpointItem[] {
  const q = opts.query.trim().toLowerCase();
  return items.filter((ep) => {
    if (opts.method !== "ALL" && !ep.methods.includes(opts.method)) return false;
    if (opts.framework !== "ALL" && ep.kind !== opts.framework) return false;
    if (opts.conflictsOnly && !ep.duplicateConflict) return false;
    if (opts.runtimeOnly && !ep.runtimeMatched) return false;
    if (q) {
      const hay = `${ep.fullPath} ${ep.handler} ${ep.controller} ${ep.resolvedUrl} ${ep.framework}`.toLowerCase();
      if (!hay.includes(q)) return false;
    }
    return true;
  });
}
