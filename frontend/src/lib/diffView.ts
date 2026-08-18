import { createTwoFilesPatch } from 'diff';

/**
 * Render a compact unified diff between two config YAML strings. Only
 * changed hunks (with 2 lines of context) are returned, so the user can
 * see exactly what a merge/load changed in the editor.
 */
export function renderYamlDiff(before: string, after: string): string {
  if (before === after) return '';
  return createTwoFilesPatch('before', 'after', before, after, '', '', {
    context: 2,
  });
}