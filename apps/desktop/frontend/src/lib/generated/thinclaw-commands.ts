// Generated command surface split. Do not hand-edit command names here.
import { commands } from "../bindings";

type ThinClawCommandName = Extract<keyof typeof commands, `thinclaw${string}`>;

export const thinclawCommands = Object.fromEntries(
  Object.entries(commands).filter(([name]) => name.startsWith("thinclaw")),
) as Pick<typeof commands, ThinClawCommandName>;
