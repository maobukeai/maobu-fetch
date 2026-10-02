import type { LanDiskInspectionResult } from "../../types";
import { formatBytes } from "../../formatters";
import { CloudSharePicker } from "./CloudSharePicker";

export function LanDiskPicker({
  shareInfo,
  selectedIds,
  onChange,
}: {
  shareInfo: LanDiskInspectionResult;
  selectedIds: Set<string>;
  onChange: (next: Set<string>) => void;
}) {
  const sizeDesc = shareInfo.totalSize > 0 ? ` · 总计 ${formatBytes(shareInfo.totalSize)}` : "";
  return (
    <CloudSharePicker
      platform="landisk"
      platformDisplayName="局域网互联 (LanDisk)"
      themeColor="#10b981"
      shareInfo={{
        title: shareInfo.rootName,
        files: shareInfo.files.map((f) => ({
          id: f.remotePath,
          name: f.name,
          kind: "drive#file",
          size: f.size,
          path: f.relativeDir ? `${f.relativeDir}/${f.name}` : f.name,
          extension: f.name.includes(".") ? f.name.split(".").pop() : undefined,
          category: "file",
          mimeType: "application/octet-stream",
        })),
        fileCount: shareInfo.fileCount,
        folderCount: shareInfo.folderCount,
        totalSize: shareInfo.totalSize,
        passCodeRequired: false,
      }}
      selectedIds={selectedIds}
      onChange={onChange}
      tipText={`💡 局域网互联目录已递归解构（共 ${shareInfo.fileCount} 项${sizeDesc}），各文件支持 16 线程 HTTP Range 并发直连与断点续传。`}
    />
  );
}
