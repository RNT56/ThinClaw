// Generated command surface split. Do not hand-edit command names here.
import { commands } from "../bindings";

type DirectCommandName = Extract<keyof typeof commands, `direct${string}`>;

export const directCommands = Object.fromEntries(
  Object.entries(commands).filter(([name]) => name.startsWith("direct")),
) as Pick<typeof commands, DirectCommandName>;
