-- BTCPC Mac Menu Bar Tray
-- Uses native NSStatusItem via AppleScript/JXA
-- Shows mining status in the menu bar, click to open settings
--
-- Run: osascript bin/tray/mac-tray.applescript &
-- Or: ./bin/btcpc-tray &

use framework "Foundation"
use framework "AppKit"
use scripting additions

-- URLs
property explorerURL : "http://localhost:4242"
property settingsURL : "http://localhost:4242/settings"

on run
	-- Create status bar item
	set statusBar to current application's NSStatusBar's systemStatusBar()
	set statusItem to statusBar's statusItemWithLength:(current application's NSVariableStatusItemLength)
	statusItem's setTitle:"BTCPC"

	-- Create menu
	set theMenu to current application's NSMenu's alloc()'s init()

	set settingsMenuItem to current application's NSMenuItem's alloc()'s initWithTitle:"Settings" action:"openSettings:" keyEquivalent:""
	set explorerMenuItem to current application's NSMenuItem's alloc()'s initWithTitle:"Explorer" action:"openExplorer:" keyEquivalent:""
	set pauseMenuItem to current application's NSMenuItem's alloc()'s initWithTitle:"Pause Mining" action:"pauseMining:" keyEquivalent:""
	set resumeMenuItem to current application's NSMenuItem's alloc()'s initWithTitle:"Resume Mining" action:"resumeMining:" keyEquivalent:""
	set quitMenuItem to current application's NSMenuItem's alloc()'s initWithTitle:"Quit" action:"terminate:" keyEquivalent:"q"

	theMenu's addItem:settingsMenuItem
	theMenu's addItem:explorerMenuItem
	theMenu's addItem:(current application's NSMenuItem's separatorItem())
	theMenu's addItem:pauseMenuItem
	theMenu's addItem:resumeMenuItem
	theMenu's addItem:(current application's NSMenuItem's separatorItem())
	theMenu's addItem:quitMenuItem

	statusItem's setMenu:theMenu

	-- Keep running
	current application's NSApp's run()
end run

on openSettings:sender
	do shell script "open " & settingsURL
end openSettings:

on openExplorer:sender
	do shell script "open " & explorerURL
end openExplorer:

on pauseMining:sender
	do shell script "curl -s -X POST " & settingsURL & " -d action=pause"
	display notification "Mining paused" with title "BTCPC"
end pauseMining:

on resumeMining:sender
	do shell script "curl -s -X POST " & settingsURL & " -d action=auto"
	display notification "Mining resumed (auto mode)" with title "BTCPC"
end resumeMining:
