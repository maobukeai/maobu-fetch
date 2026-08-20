import type { BaiduShareInfo } from "../../services/baidupan";
import { CloudSharePicker } from "./CloudSharePicker";

export function BaiduPanPicker({
  shareInfo,
  selectedIds,
  onChange,
  onVerifyPassCode,
  verifyingPassCode,
  passCodeError,
}: {
  shareInfo: BaiduShareInfo;
  selectedIds: Set<string>;
  onChange: (next: Set<string>) => void;
  onVerifyPassCode?: (passCode: string) => void;
  verifyingPassCode?: boolean;
  passCodeError?: string;
}) {
  return (
    <CloudSharePicker
      platform="baidu"
      platformDisplayName="百度网盘"
      themeColor="#2563eb"
      shareInfo={{
        title: shareInfo.title,
        files: shareInfo.files,
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
      tipText="💡 百度网盘 SVIP 账号可享受满速下载；普通账号受百度官方限速限制。"
    />
  );
}
