import { describe, expect, it } from 'vitest';
import { renderYamlDiff } from './diffView';

describe('renderYamlDiff', () => {
  it('returns empty when nothing changed', () => {
    expect(renderYamlDiff('a: 1\nb: 2\n', 'a: 1\nb: 2\n')).toBe('');
  });

  it('shows the changed line with context', () => {
    const out = renderYamlDiff(
      'machine:\n  network:\n    interfaces:\n      - interface: eno1\n        mtu: 1500\n',
      'machine:\n  network:\n    interfaces:\n      - interface: eno1\n        mtu: 9000\n',
    );
    expect(out).toContain('-        mtu: 1500');
    expect(out).toContain('+        mtu: 9000');
    expect(out).toContain('@@');
  });

  it('shows added and removed lines', () => {
    const out = renderYamlDiff('a: 1\n', 'a: 1\nb: 2\n');
    expect(out).toContain('+b: 2');
  });
});