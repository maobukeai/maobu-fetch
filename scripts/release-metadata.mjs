export function validateExeMetadata(metadata, version) {
  const productVersion = String(metadata.ProductVersion ?? "").trim();
  const original = String(metadata.OriginalFilename ?? "").trim();
  const product = String(metadata.ProductName ?? "").trim();
  if (![version, `${version}.0`].includes(productVersion)) {
    throw new Error(`EXE ProductVersion与目标版本不一致，拒绝发布：${productVersion}`);
  }
  if (!/maobu/i.test(product)) throw new Error("EXE ProductName不是Maobu Fetch，拒绝发布");
  if (original.toLowerCase() !== "maobu-fetch.exe") {
    throw new Error("EXE OriginalFilename必须为maobu-fetch.exe，空值也不允许发布");
  }
}

export function installerNameMatches(name, version, suffix) {
  const escaped=version.replace(/[.*+?^${}()|[\]\\]/g,"\\$&");
  return new RegExp(`^Maobu(?: Fetch|\\.Fetch)_${escaped}_x64${suffix === "exe" ? "-setup\\.exe" : "_zh-CN\\.msi"}$`,"i").test(name);
}
export function validateArtifactTimes(application, installers) {
  if (!Number.isFinite(application) || installers.some(time => !Number.isFinite(time) || time < application)) {
    throw new Error("安装包早于本次主程序，拒绝混合旧产物；请重新运行pnpm tauri build");
  }
}
