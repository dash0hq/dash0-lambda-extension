/** A failure the tool reports as a one-line error, without a stack trace. */
export class LayerError extends Error {}

export interface Log {
  info(message: string): void;
  warning(message: string): void;
}

/**
 * Logs to stderr, so stdout carries only a command's result. GitHub Actions
 * reads the ::warning:: workflow command from stderr too.
 */
export const stderrLog: Log = {
  info: (message) => console.error(message),
  warning: (message) => console.error(`::warning::${message}`),
};
