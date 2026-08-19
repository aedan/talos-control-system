import { parseAllDocuments as yamlParseAll } from 'yaml';

export interface NetRouteBlock {
  network: string;
  gateway: string;
  metric: string;
}

export interface NetVlanBlock {
  id: string;
  vlanId: string;
  mtu: string;
  dhcp: boolean;
  addresses: string[];
  routes: NetRouteBlock[];
}

export interface NetInterfaceBlock {
  id: string;
  interface: string;
  dhcp: boolean;
  ignore: boolean;
  mtu: string;
  addresses: string[];
  routes: NetRouteBlock[];
  bondMode: string;
  bondMembers: string;
  vlans: NetVlanBlock[];
}

export interface NetworkBuilderState {
  interfaces: NetInterfaceBlock[];
  nameservers: string[];
}

export interface NetworkBuilderKeys {
  interfaces: boolean;
  nameservers: boolean;
}

export const BOND_MODES = ['none', '802.3ad', 'active-backup', 'balance-rr', 'balance-xor', 'balance-tlb'];

/**
 * crypto.randomUUID is only available in secure contexts (HTTPS or
 * localhost). TCS is often reached over plain HTTP on a LAN address, so
 * fall back to a timestamp+random id when it is missing.
 */
export function newBlockId(): string {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return crypto.randomUUID();
  }
  return `id-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 12)}`;
}

export function newNetVlan(id: string = newBlockId()): NetVlanBlock {
  return { id, vlanId: '', mtu: '', dhcp: false, addresses: [], routes: [] };
}

export function newNetInterface(id: string = newBlockId()): NetInterfaceBlock {
  return {
    id,
    interface: '',
    dhcp: false,
    ignore: false,
    mtu: '',
    addresses: [],
    routes: [],
    bondMode: 'none',
    bondMembers: '',
    vlans: [],
  };
}

/**
 * Serialize the network builder into the YAML fragment sent to the helpers
 * endpoint. Emits `interfaces:` and/or `nameservers:` at the top level; the
 * backend nests the fragment under `machine.network` before deep-merging.
 * VLANs are emitted nested under their parent interface (`vlans:` list),
 * matching the standard Talos v1alpha1 machine config and supporting any
 * number of VLANs per interface. Only enabled keys are emitted.
 */
export function buildNetworkHelperYaml(
  state: NetworkBuilderState,
  keys: NetworkBuilderKeys,
): string {
  const lines: string[] = [];

  if (keys.interfaces) {
    const blocks = state.interfaces.filter((b) => b.interface.trim());
    if (blocks.length > 0) {
      lines.push('interfaces:');
      for (const b of blocks) {
        lines.push(`  - interface: ${b.interface.trim()}`);
        if (b.dhcp) lines.push('    dhcp: true');
        if (b.ignore) lines.push('    ignore: true');
        const mtu = b.mtu.trim();
        if (mtu) lines.push(`    mtu: ${mtu}`);
        const addresses = b.addresses.map((a) => a.trim()).filter(Boolean);
        if (addresses.length > 0) {
          lines.push('    addresses:');
          for (const a of addresses) lines.push(`      - ${a}`);
        }
        const routes = b.routes.filter((r) => r.network.trim() && r.gateway.trim());
        if (routes.length > 0) {
          lines.push('    routes:');
          for (const r of routes) {
            lines.push(`      - network: ${r.network.trim()}`);
            lines.push(`        gateway: ${r.gateway.trim()}`);
            if (r.metric) lines.push(`        metric: ${r.metric.trim()}`);
          }
        }
        if (b.bondMode !== 'none') {
          const members = b.bondMembers.split(',').map((m) => m.trim()).filter(Boolean);
          if (members.length > 0) {
            lines.push('    bond:');
            lines.push('      interfaces:');
            for (const m of members) lines.push(`        - ${m}`);
            lines.push(`      mode: ${b.bondMode}`);
          }
        }
        const vlans = b.vlans.filter((v) => v.vlanId.trim());
        if (vlans.length > 0) {
          lines.push('    vlans:');
          for (const v of vlans) {
            lines.push(`      - vlanId: ${v.vlanId.trim()}`);
            if (v.dhcp) lines.push('        dhcp: true');
            const vmtu = v.mtu.trim();
            if (vmtu) lines.push(`        mtu: ${vmtu}`);
            const vaddrs = v.addresses.map((a) => a.trim()).filter(Boolean);
            if (vaddrs.length > 0) {
              lines.push('        addresses:');
              for (const a of vaddrs) lines.push(`          - ${a}`);
            }
            const vroutes = v.routes.filter((r) => r.network.trim() && r.gateway.trim());
            if (vroutes.length > 0) {
              lines.push('        routes:');
              for (const r of vroutes) {
                lines.push(`          - network: ${r.network.trim()}`);
                lines.push(`            gateway: ${r.gateway.trim()}`);
                if (r.metric) lines.push(`            metric: ${r.metric.trim()}`);
              }
            }
          }
        }
      }
    }
  }

  if (keys.nameservers) {
    const ns = state.nameservers.map((n) => n.trim()).filter(Boolean);
    if (ns.length > 0) {
      lines.push('nameservers:');
      for (const n of ns) lines.push(`  - ${n}`);
    }
  }

  return lines.join('\n');
}

/**
 * Parse a full Talos machine config YAML into builder state, so the helper
 * blocks start from the node's current values. Tolerates the MachineConfig
 * resource wrapper (`node`/`metadata`/`spec`) where the real config is an
 * opaque YAML string inside `spec`. Nested `vlans:` on an interface are read
 * directly; standalone `kind: VLANConfig` documents are folded into their
 * parent interface's `vlans:` list (a nested entry with the same id wins).
 */
export function parseNetworkIntoBuilder(yamlText: string): NetworkBuilderState {
  const state: NetworkBuilderState = { interfaces: [], nameservers: [] };
  try {
    const docs = yamlParseAll(yamlText)
      .map((d) => d.toJS({ maxAliasCount: 1000 }) as Record<string, any> | null)
      .filter(Boolean);
    let machine = docs[0]?.machine as Record<string, any> | undefined;
    if (!machine && typeof docs[0]?.spec === 'string') {
      const specDocs = yamlParseAll(docs[0].spec)
        .map((d) => d.toJS({ maxAliasCount: 1000 }) as Record<string, any> | null)
        .filter(Boolean);
      machine = specDocs[0]?.machine as Record<string, any> | undefined;
    }
    const net = machine?.network;
    if (!net) return state;

    const vlanDocs = docs.filter((d) => d?.kind === 'VLANConfig') as Array<Record<string, any>>;

    const parseRoutes = (raw: unknown): NetRouteBlock[] =>
      Array.isArray(raw)
        ? raw
            .filter((r) => r && typeof r === 'object')
            .map((r: Record<string, unknown>) => ({
              network: typeof r.network === 'string' ? r.network : '',
              gateway: typeof r.gateway === 'string' ? r.gateway : '',
              metric: r.metric != null ? String(r.metric) : '',
            }))
        : [];

    const parseVlan = (v: Record<string, any>): NetVlanBlock => ({
      id: newBlockId(),
      vlanId: v.vlanId != null ? String(v.vlanId) : '',
      mtu: v.mtu != null ? String(v.mtu) : '',
      dhcp: Boolean(v.dhcp),
      addresses: Array.isArray(v.addresses) ? v.addresses.map(String) : [],
      routes: parseRoutes(v.routes),
    });

    const blocks: NetInterfaceBlock[] = [];
    if (Array.isArray(net.interfaces)) {
      for (const i of net.interfaces) {
        if (!i || typeof i !== 'object') continue;
        const bond = i.bond as { interfaces?: unknown; mode?: unknown } | undefined;
        const nested = Array.isArray(i.vlans)
          ? (i.vlans as Array<Record<string, any>>)
              .filter((v) => v && typeof v === 'object')
              .map(parseVlan)
          : [];
        blocks.push({
          id: newBlockId(),
          interface: typeof i.interface === 'string' ? i.interface : '',
          dhcp: Boolean(i.dhcp),
          ignore: Boolean(i.ignore),
          mtu: i.mtu != null ? String(i.mtu) : '',
          addresses: Array.isArray(i.addresses) ? i.addresses.map(String) : [],
          routes: parseRoutes(i.routes),
          bondMode: bond?.mode ? String(bond.mode) : 'none',
          bondMembers: Array.isArray(bond?.interfaces)
            ? (bond.interfaces as unknown[]).map(String).join(', ')
            : '',
          vlans: nested,
        });
      }
    }

    // Fold standalone VLANConfig docs into their parent interface.
    for (const doc of vlanDocs) {
      const parent = typeof doc.parent === 'string' ? doc.parent : '';
      const id = doc.vlanID != null ? String(doc.vlanID) : '';
      if (!parent || !id) continue;
      let block = blocks.find((b) => b.interface === parent);
      if (!block) {
        block = { ...newNetInterface(), interface: parent };
        blocks.push(block);
      }
      if (block.vlans.some((v) => v.vlanId === id)) continue;
      const addrs = Array.isArray(doc.addresses)
        ? (doc.addresses as Array<Record<string, unknown>>)
            .map((a) => (typeof a?.address === 'string' ? a.address : ''))
            .filter(Boolean)
        : [];
      const routes: NetRouteBlock[] = Array.isArray(doc.routes)
        ? (doc.routes as Array<Record<string, unknown>>)
            .filter((r) => r && typeof r === 'object')
            .map((r) => ({
              network:
                typeof r.destination === 'string' && r.destination
                  ? r.destination
                  : '0.0.0.0/0',
              gateway: typeof r.gateway === 'string' ? r.gateway : '',
              metric: r.metric != null ? String(r.metric) : '',
            }))
        : [];
      block.vlans.push({ id: newBlockId(), vlanId: id, mtu: '', dhcp: false, addresses: addrs, routes });
    }

    state.interfaces = blocks;

    if (Array.isArray(net.nameservers)) {
      state.nameservers = net.nameservers.map(String);
    }
  } catch {
    // Unparseable YAML — leave the builder empty.
  }
  return state;
}
