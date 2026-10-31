// Moving through the remote server's folder listing (ADR-0037): paths are relative to the server
// root, `.` the root itself, parts joined by `/` as the server sends them.

/** The folder `path` lies in, or null for the server root, above which nothing is listed. */
export function parentFolder(path: string): string | null {
  const parts = path.split('/').filter((p) => p !== '' && p !== '.');
  if (parts.length === 0) return null;
  return parts.length === 1 ? '.' : parts.slice(0, -1).join('/');
}
