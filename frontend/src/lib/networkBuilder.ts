import { parse as yamlParse } from 'yaml';

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
  };
}

/**
 * Serialize the network builder into the YAML fragment sent to the helpers
 * endpoint. Emits `interfaces:` and/or `nameservers:` at the top level; the
 * backend nests the fragment under `machine.network` before deep-merging.
 * Only enabled keys are emitted.
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
          }
        }
        const vlanId = b.vlanId.trim();
        if (vlanId) {
          lines.push('    vlan:');
          lines.push(`      vlanId: ${vlanId}`);
        }
        if (b.bondMode !== 'none') {
          const members = b.bondMembers.split(',').map((m) => m.trim()).filter(Boolean);
          if (members.length > 0) {
            lines.push('    bonds:');
            lines.push(`      ${b.interface.trim()}:`);
            lines.push('        interfaces:');
            for (const m of members) lines.push(`          - ${m}`);
            lines.push(`        mode: ${b.bondMode}`);
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
 * opaque YAML string inside `spec`.
 */
export function parseNetworkIntoBuilder(yamlText: string): NetworkBuilderState {
  const state: NetworkBuilderState = { interfaces: [], nameservers: [] };
  try {
    const doc = yamlText ? (yamlParse(yamlText) as Record<string, any> | null) : null;
    let machine = doc?.machine as Record<string, any> | undefined;
    if (!machine && typeof doc?.spec === 'string') {
      const spec = yamlParse(doc.spec) as Record<string, any> | null;
      machine = spec?.machine as Record<string, any> | undefined;
    }
    const net = machine?.network;
    if (!net) return state;

    if (Array.isArray(net.interfaces)) {
      state.interfaces = net.interfaces
        .filter((i: unknown) => i && typeof i === 'object')
        .map((i: Record<string, unknown>) => {
          const bonds = i.bonds as Record<string, { interfaces?: unknown; mode?: unknown }> | undefined;
          const bondKey = bonds ? Object.keys(bonds)[0] : undefined;
          const bond = bondKey && bonds ? bonds[bondKey] : undefined;
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
            vlanId: (i.vlan as { vlanId?: unknown } | undefined)?.vlanId != null
              ? String((i.vlan as { vlanId: unknown }).vlanId)
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