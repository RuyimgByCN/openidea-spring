import { describe, expect, it } from "vitest";
import { BridgeClient, filterEndpoints, layoutBeanGraph } from "../bridge";
import { getStrings } from "../i18n";
import { DEMO_SNAPSHOT, type BeanGraphEdge, type BeanItem } from "../types";

describe("Spring WebView BridgeClient & helpers", () => {
  it("completes ready -> init handshake and rpc roundtrip", async () => {
    const sent: string[] = [];
    let pushToClient: ((raw: string) => void) | null = null;

    const client = new BridgeClient({
      postMessage: (raw) => sent.push(raw),
      onMessage: (fn) => {
        pushToClient = fn;
      },
    });

    const initPromise = client.requestInit(1000);
    expect(JSON.parse(sent[0])).toEqual({ op: "ready" });
    pushToClient!(
      JSON.stringify({
        op: "init",
        locale: "zh-CN",
        pending: { tab: "endpoints" },
        snapshot: DEMO_SNAPSHOT,
      }),
    );

    const init = await initPromise;
    expect(init.locale).toBe("zh-CN");
    expect(init.pending?.tab).toBe("endpoints");

    const rpcPromise = client.rpc<{ valid: boolean }>("spel.validate", {
      expression: "#{1 + 1}",
    });
    const reqMsg = JSON.parse(sent[1]);
    expect(reqMsg.op).toBe("rpc");
    expect(reqMsg.method).toBe("spel.validate");

    pushToClient!(
      JSON.stringify({
        op: "rpc-result",
        id: reqMsg.id,
        ok: true,
        data: { valid: true },
      }),
    );
    await expect(rpcPromise).resolves.toEqual({ valid: true });
  });

  it("computes layered DAG coordinates and flags circular dependency nodes", () => {
    const beans: BeanItem[] = [
      { ...DEMO_SNAPSHOT.beans[1], id: "A", className: "AlphaService", stereotype: "service" },
      { ...DEMO_SNAPSHOT.beans[2], id: "B", className: "BetaService", stereotype: "service" },
      { ...DEMO_SNAPSHOT.beans[3], id: "C", className: "GammaRepo", stereotype: "repository" },
    ];
    const edges: BeanGraphEdge[] = [
      { fromId: "A", toId: "B", label: "beta", isCycle: true },
      { fromId: "B", toId: "A", label: "alpha", isCycle: true },
      { fromId: "B", toId: "C", label: "gamma", isCycle: false },
    ];
    const layout = layoutBeanGraph(beans, edges, [["A", "B"]]);
    expect(layout.nodes.length).toBe(3);
    expect(layout.nodes.find((n) => n.bean.id === "A")?.inCycle).toBe(true);
    expect(layout.nodes.find((n) => n.bean.id === "C")?.inCycle).toBe(false);
    expect(layout.edges.length).toBe(3);
  });

  it("filters endpoints by framework, method, and text search", () => {
    const wf = filterEndpoints(DEMO_SNAPSHOT.endpoints, {
      query: "stream",
      method: "GET",
      framework: "webFlux",
      conflictsOnly: false,
      runtimeOnly: false,
    });
    expect(wf.length).toBe(1);
    expect(wf[0].fullPath).toBe("/stream/events");
  });

  it("returns bilingual UI strings for EN and zh-CN", () => {
    expect(getStrings("en").tabs.beans).toBe("Beans & Graph");
    expect(getStrings("zh-CN").tabs.beans).toBe("Bean 与依赖图");
  });
});
