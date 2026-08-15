from __future__ import annotations

import shutil
import subprocess
import unittest
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "install.ps1"


class InstallPowerShellTests(unittest.TestCase):
    def test_security_and_usability_contracts_are_present(self) -> None:
        text = SCRIPT.read_text(encoding="utf-8")
        required = [
            "Get-FileHash",
            "SHA256SUMS",
            "Checksum verification failed",
            "[Environment]::SetEnvironmentVariable('Path'",
            "supermem.exe",
            "finally",
            "Remove-Item -LiteralPath $Temporary",
            "-UseBasicParsing",
        ]
        for marker in required:
            with self.subTest(marker=marker):
                self.assertIn(marker, text)
        self.assertNotIn("Invoke-Expression", text)
        self.assertNotIn("Start-Process powershell -Verb RunAs", text)

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
