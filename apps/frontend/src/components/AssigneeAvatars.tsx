import Avatar from "./Avatar";

const MAX_SHOWN = 3;

export default function AssigneeAvatars({ names }: { names: string[] }) {
  if (names.length === 0) {
    return <Avatar name={null} />;
  }
  const shown = names.slice(0, MAX_SHOWN);
  const overflow = names.length - shown.length;
  return (
    <span className="avatar-stack" title={names.join(", ")}>
      {shown.map((name) => (
        <Avatar key={name} name={name} />
      ))}
      {overflow > 0 ? (
        <span aria-label={`${overflow} more`} className="avatar avatar-none">
          +{overflow}
        </span>
      ) : null}
    </span>
  );
}
