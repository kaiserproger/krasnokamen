-- Fixture only: the v1 (Lua) guest body of an API 0.6.0 package.
--
-- No Lua runtime exists in the Krasnokamen core, and a package with no
-- `plugin.wasm` is refused during discovery, so this file cannot execute. It is
-- kept so the fixture directory has the same shape as the removed v1 example
-- packages (a `main.lua` beside a 0.6.0 manifest) for the startup fail-closed
-- scenario and for readers comparing the two contracts.
return {
  id = "legacy-api-0-6",
  on_command = function(command, player)
    return "legacy fixture: v1 command handling never runs"
  end,
}
