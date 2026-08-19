import { describe, expect, it } from 'vitest';
import {
  buildNetworkHelperYaml,
  newNetInterface,
  newNetVlan,
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
          routes: [{ network: '0.0.0.0/0', gateway: '192.168.1.2', metric: '200' }],
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
    expect(out).toContain('metric: 200');
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

  it('emits a bond with members and mode on the interface (bond: key)', () => {
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
    expect(out).toContain('bond:');
    expect(out).not.toContain('bonds:');
    expect(out).toContain('- eno49');
    expect(out).toContain('- eno50');
    expect(out).toContain('mode: 802.3ad');
  });

  it('emits multiple nested vlans on a single interface, each with addresses and routes', () => {
    const state: NetworkBuilderState = {
      interfaces: [
        {
          ...newNetInterface('a'),
          interface: 'bond0',
          bondMode: '802.3ad',
          bondMembers: 'eno49, eno50',
          vlans: [
            {
              ...newNetVlan('v1'),
              vlanId: '207',
              addresses: ['162.242.191.68/26'],
              routes: [{ network: '0.0.0.0/0', gateway: '162.242.191.65', metric: '100' }],
            },
            {
              ...newNetVlan('v2'),
              vlanId: '300',
              mtu: '9000',
              addresses: ['10.30.0.1/24'],
            },
          ],
        },
      ],
      nameservers: [],
    };
    const out = buildNetworkHelperYaml(state, ALL_KEYS);
    // Nested under the interface, not a standalone VLANConfig doc.
    expect(out).toContain('vlans:');
    expect(out).not.toContain('kind: VLANConfig');
    expect(out).not.toContain('---');
    expect(out).toContain('- vlanId: 207');
    expect(out).toContain('- vlanId: 300');
    expect(out).toContain('- 162.242.191.68/26');
    expect(out).toContain('gateway: 162.242.191.65');
    expect(out).toContain('metric: 100');
    expect(out).toContain('mtu: 9000');
    expect(out).toContain('- 10.30.0.1/24');
    // Both vlans present.
    expect((out.match(/- vlanId: /g) ?? []).length).toBe(2);
  });

  it('emits a non-default vlan route with destination network and metric', () => {
    const state: NetworkBuilderState = {
      interfaces: [
        {
          ...newNetInterface('a'),
          interface: 'bond0',
          vlans: [
            {
              ...newNetVlan('v1'),
              vlanId: '207',
              addresses: ['10.10.10.1/24'],
              routes: [{ network: '10.20.0.0/16', gateway: '10.10.10.254', metric: '100' }],
            },
          ],
        },
      ],
      nameservers: [],
    };
    const out = buildNetworkHelperYaml(state, ALL_KEYS);
    expect(out).toContain('- network: 10.20.0.0/16');
    expect(out).toContain('gateway: 10.10.10.254');
    expect(out).toContain('metric: 100');
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
            metric: 200
      - interface: eno2
        ignore: true
      - interface: bond0
        mtu: 1500
        bond:
          interfaces:
            - eno49
            - eno50
          mode: 802.3ad
        vlans:
          - vlanId: 207
            addresses:
              - 162.242.191.68/26
            routes:
              - network: 0.0.0.0/0
                gateway: 162.242.191.65
                metric: 100
          - vlanId: 300
            mtu: 9000
            addresses:
              - 10.30.0.1/24
    nameservers:
      - 8.8.8.8
      - 1.1.1.1
cluster:
  clusterName: demo
`;

  it('parses interfaces, bonds, nested vlans and nameservers from a machine config', () => {
    const state = parseNetworkIntoBuilder(yamlText);
    expect(state.interfaces).toHaveLength(3);
    expect(state.nameservers).toEqual(['8.8.8.8', '1.1.1.1']);

    const eno1 = state.interfaces[0];
    expect(eno1.interface).toBe('eno1');
    expect(eno1.mtu).toBe('1500');
    expect(eno1.addresses).toEqual(['192.168.1.200/24']);
    expect(eno1.routes[0]).toEqual({ network: '0.0.0.0/0', gateway: '192.168.1.2', metric: '200' });

    expect(state.interfaces[1].interface).toBe('eno2');
    expect(state.interfaces[1].ignore).toBe(true);

    const bond = state.interfaces[2];
    expect(bond.bondMode).toBe('802.3ad');
    expect(bond.bondMembers).toBe('eno49, eno50');
    expect(bond.vlans).toHaveLength(2);
    expect(bond.vlans[0].vlanId).toBe('207');
    expect(bond.vlans[0].addresses).toEqual(['162.242.191.68/26']);
    expect(bond.vlans[0].routes).toEqual([
      { network: '0.0.0.0/0', gateway: '162.242.191.65', metric: '100' },
    ]);
    expect(bond.vlans[1].vlanId).toBe('300');
    expect(bond.vlans[1].mtu).toBe('9000');
    expect(bond.vlans[1].addresses).toEqual(['10.30.0.1/24']);
  });

  it('round-trips nested vlans through the serializer', () => {
    const state = parseNetworkIntoBuilder(yamlText);
    const out = buildNetworkHelperYaml(state, ALL_KEYS);
    expect(out).toContain('- interface: bond0');
    expect(out).toContain('vlans:');
    expect(out).toContain('- vlanId: 207');
    expect(out).toContain('- vlanId: 300');
    expect(out).toContain('- 162.242.191.68/26');
    expect(out).toContain('gateway: 162.242.191.65');
    expect(out).toContain('metric: 100');
    expect(out).toContain('mtu: 9000');
    expect(out).toContain('- 10.30.0.1/24');
    // Re-parsing the serialized output (nested under machine.network) yields
    // the same two vlans.
    const wrapped = `machine:\n  network:\n${out.split('\n').map((l) => (l ? `    ${l}` : '')).join('\n')}\n`;
    const reparsed = parseNetworkIntoBuilder(wrapped);
    const bond2 = reparsed.interfaces.find((i) => i.interface === 'bond0');
    expect(bond2?.vlans.map((v) => v.vlanId)).toEqual(['207', '300']);
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

  it('folds a standalone VLANConfig doc into its parent interface', () => {
    const withStandalone = `${yamlText}
---
apiVersion: v1alpha1
kind: VLANConfig
name: bond0.400
vlanID: 400
parent: bond0
up: true
addresses:
  - address: 10.40.0.1/24
routes:
  - gateway: 10.40.0.254
`;
    const state = parseNetworkIntoBuilder(withStandalone);
    const bond = state.interfaces.find((i) => i.interface === 'bond0');
    expect(bond?.vlans.map((v) => v.vlanId)).toEqual(['207', '300', '400']);
    const v400 = bond?.vlans.find((v) => v.vlanId === '400');
    expect(v400?.addresses).toEqual(['10.40.0.1/24']);
    expect(v400?.routes).toEqual([{ network: '0.0.0.0/0', gateway: '10.40.0.254', metric: '' }]);
  });

  it('does not duplicate a vlan when nested and standalone both exist', () => {
    const dup = `${yamlText}
---
apiVersion: v1alpha1
kind: VLANConfig
name: bond0.207
vlanID: 207
parent: bond0
up: true
addresses:
  - address: 9.9.9.9/24
`;
    const state = parseNetworkIntoBuilder(dup);
    const bond = state.interfaces.find((i) => i.interface === 'bond0');
    expect(bond?.vlans.filter((v) => v.vlanId === '207')).toHaveLength(1);
    // Nested entry wins.
    expect(bond?.vlans.find((v) => v.vlanId === '207')?.addresses).toEqual(['162.242.191.68/26']);
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
    expect(state.interfaces[2].vlans).toHaveLength(2);
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
      expect(newNetVlan().id).toMatch(/^id-/);
    } finally {
      restore();
    }
  });
});
