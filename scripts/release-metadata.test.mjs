import assert from "node:assert/strict";
import { validateExeMetadata, installerNameMatches, validateArtifactTimes } from "./release-metadata.mjs";
const valid = { ProductName: "Maobu Fetch", ProductVersion: "0.10.11", OriginalFilename: "maobu-fetch.exe" };
validateExeMetadata(valid, "0.10.11");
validateExeMetadata({ ...valid, ProductVersion: "0.10.11.0", OriginalFilename: "MAOBU-FETCH.EXE" }, "0.10.11");
for (const invalid of [
  { ...valid, OriginalFilename: "" }, { ...valid, OriginalFilename: "other.exe" },
  { ...valid, ProductVersion: "0.10.110" }, { ...valid, ProductVersion: "v0.10.11-test" },
  { ...valid, ProductName: "Other App", FileDescription: "Maobu Fetch" },
]) assert.throws(() => validateExeMetadata(invalid, "0.10.11"));
console.log("Release metadata: exact version/name/OriginalFilename gates passed");

assert.ok(installerNameMatches("Maobu Fetch_0.10.11_x64-setup.exe","0.10.11","exe"));
assert.ok(installerNameMatches("Maobu Fetch_0.10.11_x64_zh-CN.msi","0.10.11","msi"));
for (const name of ["Maobu Fetch_0.10.110_x64-setup.exe","Maobu Fetch_1.0.10.11_x64-setup.exe","other_0.10.11_x64-setup.exe"]) {
  assert.equal(installerNameMatches(name,"0.10.11","exe"),false);
}
validateArtifactTimes(1000,[1000,1500]);
assert.throws(()=>validateArtifactTimes(1500,[1000,2000]));
assert.throws(()=>validateArtifactTimes(Number.NaN,[2000]));
console.log("Release artifacts: exact asset names and stale-installer gates passed");
