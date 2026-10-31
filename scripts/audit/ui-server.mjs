import net from "node:net";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { uiHarness } from "./ui-harness.mjs";

export async function createAuditServer(output) {
  const root = fileURLToPath(new URL("../../", import.meta.url));
const reservation = net.createServer();
await new Promise(resolve => reservation.listen(0, "127.0.0.1", resolve));
const auditPort = reservation.address().port;
await new Promise(resolve => reservation.close(resolve));
return await createServer({ root, cacheDir: path.join(root, "node_modules", ".cache", path.basename(output)),
  server: { host: "127.0.0.1", port: auditPort, strictPort: true, watch: { ignored: ["**/src-tauri/target/**", "**/extension/dist/**", "**/dist/**", "**/docs/audits/**"] } }, plugins: [{
  name: "local-audit-harness",
  configureServer(vite) {
    vite.middlewares.use(async (req, res, next) => {
      if (!req.url?.startsWith("/__audit.html") || req.url.includes("html-proxy")) return next();
      try {
        res.setHeader("Content-Type", "text/html; charset=utf-8");
        res.end(await vite.transformIndexHtml("/__audit.html", uiHarness));
      } catch (error) { res.statusCode = 500; res.end(String(error)); }
    });
  },
}] });
}
