// Initials for an avatar: first letters of the first and last name parts.
function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  if (parts.length === 0) {
    return "?";
  }
  if (parts.length === 1) {
    return parts[0].slice(0, 2).toUpperCase();
  }
  return (parts[0][0] + parts[parts.length - 1][0]).toUpperCase();
}

// Stable hue per name so the same person always gets the same color.
function hueFor(name: string): number {
  let hash = 0;
  for (let index = 0; index < name.length; index += 1) {
    hash = (hash * 31 + name.charCodeAt(index)) % 360;
  }
  return hash;
}

export default function Avatar({ name }: { name: string | null }) {
  if (!name) {
    return (
      <span aria-label="Unassigned" className="avatar avatar-none" title="Unassigned">
        —
      </span>
    );
  }
  const hue = hueFor(name);
  return (
    <span
      aria-label={name}
      className="avatar"
      style={{ backgroundColor: `hsl(${hue}, 55%, 42%)` }}
      title={name}
    >
      {initials(name)}
    </span>
  );
}
