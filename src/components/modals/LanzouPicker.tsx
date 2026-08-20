import type { LanzouShareInfo } from "../../types";
import { CloudSharePicker } from "./CloudSharePicker";

export function LanzouPicker({
  shareInfo,
  selectedIds,
  onChange,
  onVerifyPassCode,
  verifyingPassCode,
  passCodeError,
}: {
  shareInfo: LanzouShareInfo;
  selectedIds: Set<string>;
  onChange: (next: Set<string>) => void;
  onVerifyPassCode?: (passCode: string) => void;
  verifyingPassCode?: boolean;
  passCodeError?: string;
}) {
  const totalSize = shareInfo.files.reduce((acc, f) => acc + (f.size || 0), 0);
  const fileCount = shareInfo.files.filter((f) => f.kind !== "folder").length;
  const folderCount = shareInfo.files.filter((f) => f.kind === "folder").length;

  return (
    <CloudSharePicker
      platform="lanzou"
      platformDisplayName="蓝奏云"
      themeColor="#0284c7"
      shareInfo={{
        title: shareInfo.title,
        files: shareInfo.files.map((f) => ({
          id: f.id,
          name: f.name,
          kind: f.kind === "folder" ? "drive#folder" : "drive#file",
          size: f.size,
          path: f.name,
          extension: f.name.includes(".") ? f.name.split(".").pop() : undefined,
          category: "file",
          mimeType: "application/octet-stream",
        })),
        fileCount,
        folderCount,
        totalSize,
        passCodeRequired: shareInfo.requires_password,
      }}
      selectedIds={selectedIds}
      onChange={onChange}
      onVerifyPassCode={onVerifyPassCode}
      verifyingPassCode={verifyingPassCode}
      passCodeError={passCodeError}
      tipText="💡 蓝奏云支持 100% 免登录极速解析，猫步下载器已自动启用 32 线程满载多并发加速。"
    />
  );
}
