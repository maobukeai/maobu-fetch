import os from "node:os";
import path from "node:path";

export function releaseRustFlags(environment, root, home = os.homedir()) {
  const previous = environment.CARGO_ENCODED_RUSTFLAGS !== undefined
    ? environment.CARGO_ENCODED_RUSTFLAGS.split("\x1f").filter(Boolean)
    : (environment.RUSTFLAGS ?? "").trim().split(/\s+/).filter(Boolean);
  const cargo = environment.CARGO_HOME || path.join(home, ".cargo");
  // rustc 从后向前匹配：具体工程路径优先于主目录。兼容斜杠两种拼写。
  const mappings = [[home,"/build-user"],[cargo,"/cargo"],[root,"/maobu-fetch"]];
  return [...previous, ...mappings.flatMap(([source,target]) =>
    [...new Set([path.resolve(source),path.resolve(source).replaceAll("\\","/")])]
      .map(prefix => `--remap-path-prefix=${prefix}=${target}`))];
}
