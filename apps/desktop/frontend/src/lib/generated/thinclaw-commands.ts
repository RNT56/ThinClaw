// Generated command surface split. Do not hand-edit command names here.
//
// Runtime calls must stay behind commandClient so BridgeError Result values
// become rejected promises instead of being mistaken for successful data.
import { commandClient, type CommandClient } from "../command-client";

type ThinClawCommandName = Extract<keyof CommandClient, `thinclaw${string}`>;

export const thinclawCommands = Object.fromEntries(
  Object.entries(commandClient).filter(([name]) => name.startsWith("thinclaw")),
) as Pick<CommandClient, ThinClawCommandName>;
