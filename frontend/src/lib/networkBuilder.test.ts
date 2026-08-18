import { describe, expect, it } from 'vitest';
import {
  buildNetworkHelperYaml,
  newNetInterface,
  parseNetworkIntoBuilder,
  type NetworkBuilderState,
} from './networkBuilder';

const ALL_KEYS = { interfaces: true, nameservers: true };

describe('buildNetworkHelperYaml', () => {
  it('emits nothing when keys are disabled', () => {
    const state: NetworkBuilderState = { interfaces: [], nameservers: [] };
    expect(buildNetworkHelperYaml(state, { interfaces: false, nameservers: false })).toBe('');
  });

  it('emits an interfaces block with dhcp, mtu, addresses and routes', () => {
    const state: NetworkBuilderState = {
      interfaces: [
        {
          ...newNetInterface('a'),
          interface: 'eno1',
          dhcp: true,
          mtu: '1500',
          addresses: ['192.168.1.200/24'],
          routes: [{ network: '0.0.0.0/0', gateway: '192.168.1.2' }],
        },
      ],
      nameservers: [],
    };
    const out = buildNetworkHelperYaml(state, ALL_KEYS);
    expect(out).toContain('interfaces:');
    expect(out).toContain('- interface: eno1');
    expect(out).toContain('dhcp: true');
    expect(out).toContain('mtu: 1500');
    expect(out).toContain('- 192.168.1.200/24');
    expect(out).toContain('- network: 0.0.0.0/0');
    expect(out).toContain('gateway: 192.168.1.2');
  });

  it('emits ignore and skips blocks with an empty interface name', () => {
    const state: NetworkBuilderState = {
      interfaces: [
        { ...newNetInterface('a'), interface: 'eno2', ignore: true },
        newNetInterface('b'),
      ],
      nameservers: [],
    };
    const out = buildNetworkHelperYaml(state, ALL_KEYS);
    expect(out).toContain('- interface: eno2');
    expect(out).toContain('ignore: true');
    expect((out.match(/- interface: /g) ?? []).length).toBe(1);
  });

  it('emits a bond with members and mode keyed by the interface name', () => {
    const state: NetworkBuilderState = {
      interfaces: [
        {
          ...newNetInterface('a'),
          interface: 'bond0',
          bondMode: '802.3ad',
          bondMembers: 'eno49, eno50',
        },
      ],
      nameservers: [],
    };
    const out = buildNetworkHelperYaml(state, ALL_KEYS);
    expect(out).toContain('bonds:');
    expect(out).toContain('bond0:');
    expect(out).toContain('- eno49');
    expect(out).toContain('- eno50');
    expect(out).toContain('mode: 802.3ad');
  });

  it('emits a vlan id when set', () => {
    const state: NetworkBuilderState = {
      interfaces: [{ ...newNetInterface('a'), interface: 'eno49', vlanId: '207' }],
      nameservers: [],
    };
    const out = buildNetworkHelperYaml(state, ALL_KEYS);
    expect(out).toContain('vlan:');
    expect(out).toContain('vlanId: 207');
  });

  it('emits nameservers', () => {
    const state: NetworkBuilderState = {
      interfaces: [],
      nameservers: ['8.8.8.8', '1.1.1.1'],
    };
    const out = buildNetworkHelperYaml(state, ALL_KEYS);
    expect(out).toContain('nameservers:');
    expect(out).toContain('- 8.8.8.8');
    expect(out).toContain('- 1.1.1.1');
  });
});

describe('parseNetworkIntoBuilder', () => {
  const yamlText = `
version: v1alpha1
machine:
  type: controlplane
  network:
    hostname: 914333-infra01
    interfaces:
      - interface: eno1
        mtu: 1500
        addresses:
          - 192.168.1.200/24
        routes:
          - network: 0.0.0.0/0
            gateway: 192.168.1.2
      - interface: eno2
        ignore: true
      - interface: bond0
        addresses:
          - 10.0.0.1/24
        bonds:
          bond0:
            interfaces:
              - eno49
              - eno50
            mode: 802.3ad
    nameservers:
      - 8.8.8.8
      - 1.1.1.1
cluster:
  clusterName: demo
`;

  it('parses interfaces, bonds and nameservers from a machine config', () => {
    const state = parseNetworkIntoBuilder(yamlText);
    expect(state.interfaces).toHaveLength(3);
    expect(state.nameservers).toEqual(['8.8.8.8', '1.1.1.1']);

    const eno1 = state.interfaces[0];
    expect(eno1.interface).toBe('eno1');
    expect(eno1.mtu).toBe('1500');
    expect(eno1.addresses).toEqual(['192.168.1.200/24']);
    expect(eno1.routes[0]).toEqual({ network: '0.0.0.0/0', gateway: '192.168.1.2' });

    expect(state.interfaces[1].interface).toBe('eno2');
    expect(state.interfaces[1].ignore).toBe(true);

    const bond = state.interfaces[2];
    expect(bond.bondMode).toBe('802.3ad');
    expect(bond.bondMembers).toBe('eno49, eno50');
  });

  it('round-trips through the serializer', () => {
    const state = parseNetworkIntoBuilder(yamlText);
    const out = buildNetworkHelperYaml(state, ALL_KEYS);
    expect(out).toContain('- interface: eno1');
    expect(out).toContain('- interface: eno2');
    expect(out).toContain('- interface: bond0');
    expect(out).toContain('mode: 802.3ad');
    expect(out).toContain('- 8.8.8.8');
  });

  it('returns an empty state for garbage or missing network', () => {
    expect(parseNetworkIntoBuilder('not: [valid').interfaces).toEqual([]);
    expect(parseNetworkIntoBuilder('').interfaces).toEqual([]);
    expect(parseNetworkIntoBuilder('machine:\n  type: worker').nameservers).toEqual([]);
  });

  it('unwraps a MachineConfig resource wrapper (node/metadata/spec)', () => {
    const wrapper = `node: 192.168.1.200
metadata:
  namespace: config
  type: MachineConfigs.config.talos.dev
  id: v1alpha1
spec: |-
${yamlText
  .split('\n')
  .map((l) => (l ? `  ${l}` : ''))
  .join('\n')}
`;
    const state = parseNetworkIntoBuilder(wrapper);
    expect(state.interfaces).toHaveLength(3);
    expect(state.interfaces[0].interface).toBe('eno1');
    expect(state.interfaces[0].mtu).toBe('1500');
    expect(state.interfaces[2].bondMode).toBe('802.3ad');
    expect(state.nameservers).toEqual(['8.8.8.8', '1.1.1.1']);
  });

  it('parses without crypto.randomUUID (insecure http context)', () => {
    const original = globalThis.crypto;
    const restore = () => {
      Object.defineProperty(globalThis, 'crypto', {
        value: original,
        configurable: true,
      });
    };
    try {
      Object.defineProperty(globalThis, 'crypto', {
        value: {},
        configurable: true,
      });
      const state = parseNetworkIntoBuilder(yamlText);
      expect(state.interfaces).toHaveLength(3);
      expect(state.interfaces[0].interface).toBe('eno1');
      expect(state.nameservers).toEqual(['8.8.8.8', '1.1.1.1']);
      expect(newNetInterface().id).toMatch(/^id-/);
    } finally {
      restore();
    }
  });
});