import { parseAllDocuments as yamlParseAll } from 'yaml';

export interface NetRouteBlock {
  network: string;
  gateway: string;
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
  vlanId: string;
  vlanAddresses: string;
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
    vlanId: '',
    vlanAddresses: '',
  };
}

/**
 * Serialize the network builder into the YAML fragment sent to the helpers
 * endpoint. Emits `interfaces:` and/or `nameservers:` at the top level; the
 * backend nests the fragment under `machine.network` before deep-merging.
 * VLAN blocks emit a standalone `kind: VLANConfig` document after the
 * fragment (`---` separator). Only enabled keys are emitted.
 */
export function buildNetworkHelperYaml(
  state: NetworkBuilderState,
  keys: NetworkBuilderKeys,
): string {
  const lines: string[] = [];
  const vlanDocs: string[] = [];

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
        const vlanId = b.vlanId.trim();
        if (vlanId) {
          vlanDocs.push(buildVlanConfigDoc(b.interface.trim(), vlanId, b.vlanAddresses));
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

  const fragment = lines.join('\n');
  if (!fragment) return vlanDocs.join('\n---\n');
  if (vlanDocs.length === 0) return fragment;
  return `${fragment}\n---\n${vlanDocs.join('\n---\n')}`;
}

/**
 * Talos 1.13+ configures VLANs with a standalone `kind: VLANConfig`
 * document; the vlan interface is named `<parent>.<vlanId>`.
 */
export function buildVlanConfigDoc(parent: string, vlanId: string, vlanAddresses: string): string {
  const doc = [
    'apiVersion: v1alpha1',
    'kind: VLANConfig',
    `name: ${parent}.${vlanId}`,
    `vlanID: ${vlanId}`,
    `parent: ${parent}`,
  ];
  const addrs = vlanAddresses
    .split(',')
    .map((a) => a.trim())
    .filter(Boolean);
  if (addrs.length > 0) {
    doc.push('addresses:');
    for (const a of addrs) doc.push(`  - address: ${a}`);
  }
  return doc.join('\n');
}

/**
 * Parse a full Talos machine config YAML into builder state, so the helper
 * blocks start from the node's current values. Tolerates the MachineConfig
 * resource wrapper (`node`/`metadata`/`spec`) where the real config is an
 * opaque YAML string inside `spec`, and standalone network config docs
 * (`kind: VLANConfig`) appended after the `---` separator.
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
    const vlanDocs = docs.filter((d) => d?.kind === 'VLANConfig') as Array<
      Record<string, any>
    >;
    const net = machine?.network;
    if (!net) return state;

    if (Array.isArray(net.interfaces)) {
      state.interfaces = net.interfaces
        .filter((i: unknown) => i && typeof i === 'object')
        .map((i: Record<string, unknown>) => {
          const bond = i.bond as { interfaces?: unknown; mode?: unknown } | undefined;
          const vlan = vlanDocs.find((v) => v.parent === i.interface);
          return {
            id: newBlockId(),
            interface: typeof i.interface === 'string' ? i.interface : '',
            dhcp: Boolean(i.dhcp),
            ignore: Boolean(i.ignore),
            mtu: i.mtu != null ? String(i.mtu) : '',
            addresses: Array.isArray(i.addresses) ? i.addresses.map(String) : [],
            routes: Array.isArray(i.routes)
              ? i.routes
                  .filter((r: unknown) => r && typeof r === 'object')
                  .map((r: Record<string, unknown>) => ({
                    network: typeof r.network === 'string' ? r.network : '',
                    gateway: typeof r.gateway === 'string' ? r.gateway : '',
                  }))
              : [],
            bondMode: bond?.mode ? String(bond.mode) : 'none',
            bondMembers: Array.isArray(bond?.interfaces)
              ? (bond.interfaces as unknown[]).map(String).join(', ')
              : '',
            vlanId: vlan?.vlanID != null ? String(vlan.vlanID) : '',
            vlanAddresses: Array.isArray(vlan?.addresses)
              ? (vlan.addresses as Array<Record<string, unknown>>)
                  .map((a) => (typeof a?.address === 'string' ? a.address : ''))
                  .filter(Boolean)
                  .join(', ')
              : '',
          };
        });
    }

    if (Array.isArray(net.nameservers)) {
      state.nameservers = net.nameservers.map(String);
    }
  } catch {
    // Unparseable YAML — leave the builder empty.
  }
  return state;
}