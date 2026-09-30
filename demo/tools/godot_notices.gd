## Writes the Godot engine's licence notices, as compiled into the running binary, to a
## text file. scripts/export-study.sh runs it with the 4.7.2 editor binary for the study
## build's THIRD-PARTY-NOTICES.txt:
##
##   godot --headless --path PROJECT --script /abs/demo/tools/godot_notices.gd -- --out FILE
##
## Godot generates this data from its full COPYRIGHT.txt, which covers every platform, so
## the editor and the Linux and Windows export templates of one release carry the same
## data. (The templates themselves cannot run it: official builds disable --script.)
extends SceneTree


func _initialize() -> void:
	var args := OS.get_cmdline_user_args()
	var i := args.find("--out")
	if i < 0 or i + 1 >= args.size():
		printerr("godot_notices: missing --out FILE")
		quit(2)
		return
	var text := "Godot Engine %s\n\n%s\n" % [Engine.get_version_info().string, Engine.get_license_text()]
	text += "\n\nThird-party components in the Godot Engine\n==========================================\n"
	for c: Dictionary in Engine.get_copyright_info():
		text += "\n%s\n" % c.name
		for part: Dictionary in c.parts:
			for holder in part.copyright:
				text += "  Copyright %s\n" % holder
			text += "  License: %s\n" % part.license
	text += "\n\nLicence texts\n=============\n"
	var licences := Engine.get_license_info()
	var names := licences.keys()
	names.sort()
	for n in names:
		text += "\n--- %s ---\n\n%s\n" % [n, licences[n]]
	var f := FileAccess.open(args[i + 1], FileAccess.WRITE)
	if f == null:
		printerr("godot_notices: cannot write %s" % args[i + 1])
		quit(1)
		return
	f.store_string(text)
	f.close()
	quit(0)
