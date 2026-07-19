const sourceUrl = process.argv[2];
const counts = (process.argv[3] ?? "1,4,8,16")
  .split(",")
  .map(Number)
  .filter((value) => Number.isInteger(value) && value > 0);

if (!sourceUrl) {
  throw new Error("Usage: node scripts/parallel_range_bench.mjs <url> [counts]");
}

const baseOffset = 4_171_333_632;
const rangeBytes = 8 * 1024 * 1024;
for (const count of counts) {
  const began = performance.now();
  const sizes = await Promise.all(
    Array.from({ length: count }, async (_, index) => {
      const start = baseOffset + index * 2 * rangeBytes;
      const response = await fetch(sourceUrl, {
        headers: {
          "accept-encoding": "identity",
          range: `bytes=${start}-${start + rangeBytes - 1}`,
        },
      });
      if (response.status !== 206) throw new Error(`HTTP ${response.status}`);
      return (await response.arrayBuffer()).byteLength;
    }),
  );
  const seconds = (performance.now() - began) / 1000;
  const bytes = sizes.reduce((sum, value) => sum + value, 0);
  console.log(
    JSON.stringify({
      parallel: count,
      seconds: Number(seconds.toFixed(2)),
      mibps: Number((bytes / 1024 / 1024 / seconds).toFixed(1)),
    }),
  );
}
