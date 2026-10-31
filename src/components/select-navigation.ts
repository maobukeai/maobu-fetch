export function nextEnabledIndex(options: readonly { disabled?: boolean }[], current: number, direction: 1 | -1): number {
  if (!options.length) return -1;
  for (let step = 1; step <= options.length; step++) {
    const index = (current + direction * step + options.length * 2) % options.length;
    if (!options[index].disabled) return index;
  }
  return -1;
}
