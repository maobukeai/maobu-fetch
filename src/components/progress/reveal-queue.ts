/** Serializes lightweight hover requests independently of task/pin operations. */
export function createRevealQueue<T>(
  send: (revealed: boolean) => Promise<T>,
  accept: (value: T) => void,
  failed: () => void,
  initialLastSent: boolean | null = false,
) {
  let active = true;
  let running = false;
  let desired = false;
  let lastSent: boolean | null = initialLastSent;
  let attempted = false;

  const flush = async () => {
    if (running || !active) return;
    running = true;
    try {
      while (active && desired !== lastSent) {
        const target = desired;
        attempted = target;
        try {
          const snapshot = await send(target);
          lastSent = target;
          if (active) accept(snapshot);
        } catch {
          lastSent = null;
          if (active) failed();
          break;
        }
      }
    } finally {
      running = false;
      // If intent changed while a failed request was pending, still send the new intent.
      if (active && desired !== attempted && desired !== lastSent) void flush();
    }
  };

  return {
    request(value: boolean) { desired = value; void flush(); },
    adopt(value: boolean) { if (!running) lastSent = value; },
    dispose() { active = false; },
  };
}
