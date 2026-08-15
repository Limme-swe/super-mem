from __future__ import annotations

import shutil
import subprocess
import unittest
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "uninstall.ps1"


class UninstallPowerShellTests(unittest.TestCase):
    def test_destructive_actions_require_confirmation(self) -> None:
        text = SCRIPT.read_text(encoding="utf-8")
        self.assertIn("$PurgeData -and -not $Yes", text)
        self.assertIn("Memories cannot be recovered".lower(), text.lower())
        self.assertIn("SUPER_MEM_DB was not removed", text)
        self.assertIn("-LiteralPath", text)
        self.assertIn("$DryRun", text)
        self.assertNotIn("Remove-Item $env:LOCALAPPDATA", text)

    def test_script_parses_when_powershell_is_available(self) -> None:
        executable = shutil.which("pwsh") or shutil.which("powershell")
        if executable is None:
            self.skipTest("PowerShell is not installed on this platform")
        command = (
            "$errors = $null; "
            f"[void][System.Management.Automation.Language.Parser]::ParseFile('{SCRIPT}', [ref]$null, [ref]$errors); "
            "if ($errors.Count -gt 0) { $errors | Out-String | Write-Error; exit 1 }"
        )
        result = subprocess.run(
            [executable, "-NoProfile", "-NonInteractive", "-Command", command],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
