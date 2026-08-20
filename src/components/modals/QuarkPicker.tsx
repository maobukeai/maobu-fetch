import type { QuarkShareInfo } from "../../services/quark";
import { CloudSharePicker } from "./CloudSharePicker";

export function QuarkPicker({
  shareInfo,
  selectedIds,
  onChange,
  onVerifyPassCode,
  verifyingPassCode,
  passCodeError,
}: {
  shareInfo: QuarkShareInfo;
  selectedIds: Set<string>;
  onChange: (next: Set<string>) => void;
  onVerifyPassCode?: (passCode: string) => void;
  verifyingPassCode?: boolean;
  passCodeError?: string;
}) {
  return (
    <CloudSharePicker
      platform="quark"
      platformDisplayName="夸克网盘"
      themeColor="#d97706"
      shareInfo={{
        title: shareInfo.title,
        files: shareInfo.files.map((f) => ({
          id: f.id,
          name: f.name,
          kind: f.kind,
          size: f.size,
          path: f.path,
          extension: f.file_extension,
          category: f.format_type,
          mimeType: f.format_type,
        })),
        fileCount: shareInfo.file_count,
        folderCount: shareInfo.folder_count,
        totalSize: shareInfo.total_size,
        passCodeRequired: shareInfo.pass_code_required,
      }}
      selectedIds={selectedIds}
      onChange={onChange}
      onVerifyPassCode={onVerifyPassCode}
      verifyingPassCode={verifyingPassCode}
      passCodeError={passCodeError}
      tipText="💡 夸克 SVIP 账号可享受最高 50MB/s 极速下载；普通账号受夸克官方带宽限制。"
    />
  );
}
