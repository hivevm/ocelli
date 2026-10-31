// Avatars are loaded from Gravatar only once the developer turns them on, in the desktop
// application and in a browser alike (ADR-0025, ADR-0035): until then nothing is asked of anyone
// but the host.

/** Whether avatars are on, from what the browser stored: off unless the developer turned them on. */
export function avatarsOn(stored: string | null): boolean {
  return stored === 'on';
}
