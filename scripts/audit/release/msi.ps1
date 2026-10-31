function Get-AuditMsiInventory([string]$Path) {
    $installer=$null;$database=$null;$view=$null
    try {
        $installer=New-Object -ComObject WindowsInstaller.Installer
        # Mode 0 is a read-only database. No installation, repair, registration or custom action runs.
        $database=$installer.OpenDatabase($Path,0)
        $view=$database.OpenView('SELECT `File`, `FileName`, `FileSize`, `Version` FROM `File`')
        [void]$view.Execute()
        $files=@(while($null -ne ($record=$view.Fetch())) {
            try {
                [pscustomobject]@{Id=$record.StringData(1);Name=($record.StringData(2).Split('|')[-1]);Size=$record.IntegerData(3);Version=$record.StringData(4)}
            } finally {[void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($record)}
        })
        [void]$view.Close();[void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($view);$view=$null
        $view=$database.OpenView('SELECT `Property`, `Value` FROM `Property`')
        [void]$view.Execute();$properties=@{}
        while($null -ne ($record=$view.Fetch())) {
            try {$properties[$record.StringData(1)]=$record.StringData(2)} finally {[void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($record)}
        }
        [void]$view.Close()
        return [pscustomobject]@{Files=$files;Properties=$properties}
    } finally {
        foreach($object in @($view,$database,$installer)) {if($null -ne $object){[void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($object)}}
    }
}
