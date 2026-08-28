interface Props {
  readonly dragOver: boolean;
  readonly onOpen: () => void;
}

export function EmptyState({ dragOver, onOpen }: Props) {
  return (
    <div className="center">
      <div className={dragOver ? "dropzone over" : "dropzone"}>
        <h1>Open a video to whittle</h1>
        <p>
          Drop a file here, or pick one below. Nothing is written until you
          export.
        </p>
        <button className="primary" onClick={onOpen}>
          Choose file…
        </button>
      </div>
    </div>
  );
}
