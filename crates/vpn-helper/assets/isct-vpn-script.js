// openconnect route/DNS hook for the campus VPN (Windows).
//
// openconnect runs hooks only through cscript, so this JScript only starts
// the helper next to it with openconnect's variables, which the child
// inherits; `isct-vpn-helper.exe hook` installs the campus route and the
// resolver rule (crates/vpn-helper/src/windows_net.rs). Its exit code is
// passed on for openconnect to log. An uncaught error here would end cscript
// with 0, so a helper that cannot be started is reported as 2.
var shell = WScript.CreateObject("WScript.Shell");
var dir = WScript.ScriptFullName.replace(/\\[^\\]*$/, "");
try {
  WScript.Quit(shell.Run('"' + dir + '\\isct-vpn-helper.exe" hook', 0, true));
} catch (error) {
  WScript.Quit(2);
}
