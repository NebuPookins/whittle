import type { ToolStatus } from "../types";

interface Props {
  readonly tools: ToolStatus;
  readonly onRetry: () => void;
}

const Tool = ({ name, version }: { name: string; version: string | null }) => (
  <li>
    {version === null ? (
      <>
        <span className="missing">✗</span>
        <span>{name}</span>
        <span className="missing">not found on $PATH</span>
      </>
    ) : (
      <>
        <span className="found">✓</span>
        <span>{name}</span>
        <span style={{ color: "var(--muted-dim)" }}>{version}</span>
      </>
    )}
  </li>
);

/**
 * Shown when ffmpeg or ffprobe is missing. Whittle shells out to both, so
 * there is nothing useful it can do until they are installed.
 */
export function SetupScreen({ tools, onRetry }: Props) {
  return (
    <div className="center">
      <div className="card">
        <h1>ffmpeg is required</h1>
        <p>
          Whittle drives the system <code>ffmpeg</code> and <code>ffprobe</code>{" "}
          binaries to read and trim video. They are not bundled, and neither one
          was found on your <code>$PATH</code>.
        </p>

        <h2>Detected</h2>
        <ul className="tool-list">
          <Tool name="ffmpeg" version={tools.ffmpeg} />
          <Tool name="ffprobe" version={tools.ffprobe} />
        </ul>

        <h2>Install it</h2>
        <pre>
          {[
            "Arch      sudo pacman -S ffmpeg",
            "Debian    sudo apt install ffmpeg",
            "Fedora    sudo dnf install ffmpeg",
            "openSUSE  sudo zypper install ffmpeg",
          ].join("\n")}
        </pre>
        <p style={{ marginTop: 14 }}>
          Both binaries ship in the same package, so one install covers it.
        </p>

        <button className="primary" onClick={onRetry}>
          Check again
        </button>
      </div>
    </div>
  );
}
