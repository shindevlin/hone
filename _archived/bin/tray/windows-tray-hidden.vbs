' HONE Windows Tray — Hidden launcher
' Starts the PowerShell tray script with NO console window at all.
' Place this in shell:startup to autostart on login.
'
' Usage: double-click this file, or add to startup folder

Set WshShell = CreateObject("WScript.Shell")
scriptDir = CreateObject("Scripting.FileSystemObject").GetParentFolderName(WScript.ScriptFullName)
WshShell.Run "powershell -ExecutionPolicy Bypass -WindowStyle Hidden -File """ & scriptDir & "\windows-tray.ps1""", 0, False
