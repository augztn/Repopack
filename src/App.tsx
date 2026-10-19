import { useEffect, useRef, useState, type FormEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";

type LogEvent = { level: string; message: string };
type SourceInfo = {
  name: string;
  rootPath: string;
  projectType: string;
  indicators: string[];
  nextStep: string;
};

const phases = [
  { title: "Source", detail: "GitHub URL or local ZIP" },
  { title: "Extract", detail: "Safe local workspace" },
  { title: "Detect", detail: "Framework and build markers" },
  { title: "Build", detail: "Approval-based runner (next milestone)" },
];

function ClockTime() {
  return new Date().toLocaleTimeString([], { hour12: false });
}

export default function App() {
  const [sourceInput, setSourceInput] = useState("");
  const [zipPath, setZipPath] = useState("");
  const [busy, setBusy] = useState(false);
  const [phase, setPhase] = useState("Ready");
  const [result, setResult] = useState<SourceInfo | null>(null);
  const [logs, setLogs] = useState<string[]>([
    "[system] Repopack local workspace initialized.",
    "[system] Build scripts are not executed during source inspection.",
  ]);
  const logEnd = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    listen<LogEvent>("job-log", (event) => {
      const entry = `[${event.payload.level}] ${event.payload.message}`;
      setLogs((current) => [...current.slice(-499), entry]);
    }).then((stop) => {
      if (disposed) stop();
      else unlisten = stop;
    }).catch((error: unknown) => {
      setLogs((current) => [...current, `[warning] Could not attach to job events: ${String(error)}`]);
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    logEnd.current?.scrollIntoView({ behavior: "smooth", block: "end" });
  }, [logs]);

  async function chooseZip() {
    try {
      const selected = await open({
        multiple: false,
        directory: false,
        title: "Choose a GitHub source ZIP",
        filters: [{ name: "ZIP archive", extensions: ["zip"] }],
      });
      if (typeof selected === "string") {
        setZipPath(selected);
        setSourceInput(selected);
        setResult(null);
      }
    } catch (error) {
      setLogs((current) => [...current, `[error] File picker failed: ${String(error)}`]);
    }
  }

  async function inspectSource(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const input = sourceInput.trim();
    if (!input) {
      setPhase("Input required");
      return;
    }
    setBusy(true);
    setResult(null);
    setPhase("Downloading / extracting");
    setLogs((current) => [...current, `[ui] ${ClockTime()} — Preparing source…`]);
    try {
      const prepared = await invoke<SourceInfo>("prepare_source", { input });
      setResult(prepared);
      setPhase("Inspection complete");
      setLogs((current) => [
        ...current,
        `[success] Project detected: ${prepared.projectType}`,
        `[info] Workspace: ${prepared.rootPath}`,
      ]);
    } catch (error) {
      setPhase("Failed");
      setLogs((current) => [...current, `[error] ${String(error)}`]);
    } finally {
      setBusy(false);
    }
  }

  const detectedColor = result ? "ready" : "neutral";

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-mark"><span>R</span></div>
          <div><strong>repopack</strong><small>LOCAL BUILD STUDIO</small></div>
        </div>
        <div className="nav-label">WORKSPACE</div>
        <button className="nav-item active" type="button">
          <span className="nav-icon">⌘</span><span>Build workspace</span><span className="nav-indicator" />
        </button>
        <button className="nav-item muted" type="button" disabled>
          <span className="nav-icon">◷</span><span>Job history</span><span className="soon">SOON</span>
        </button>
        <div className="sidebar-bottom">
          <div className="security-card">
            <span className="security-dot" />
            <div><strong>Local-first</strong><p>Source stays on this device.</p></div>
          </div>
          <div className="version-line"><span>Preview build</span><span>v0.1.0</span></div>
        </div>
      </aside>

      <main className="main-content">
        <header className="topbar">
          <div className="breadcrumb">Workspace <span>/</span> <strong>New build</strong></div>
          <div className="local-badge"><span /> LOCAL MODE</div>
        </header>

        <div className="content-wrap">
          <section className="page-intro">
            <div className="eyebrow"><span className="eyebrow-line" /> REPOSITORY TOOLCHAIN</div>
            <h1>From source to <span>standalone.</span></h1>
            <p>Bring a repository into your local workspace. Repopack inspects its framework and prepares the right build workflow for your machine.</p>
          </section>

          <section className="source-card panel">
            <div className="section-heading">
              <div className="section-icon">↳</div>
              <div><h2>Choose a source</h2><p>Paste a public GitHub URL or select a downloaded ZIP archive.</p></div>
              <span className="step-number">01 / 03</span>
            </div>
            <form onSubmit={inspectSource}>
              <label htmlFor="source-url">GITHUB REPOSITORY OR LOCAL ZIP PATH</label>
              <div className="source-input-row">
                <div className="input-wrap"><span className="input-prefix">↗</span><input id="source-url" value={sourceInput} onChange={(event) => { setSourceInput(event.target.value); setZipPath(""); setResult(null); }} placeholder="https://github.com/owner/repository" autoComplete="url" spellCheck={false} /></div>
                <button className="button button-secondary" type="button" onClick={chooseZip}>＋ <span>Select ZIP</span></button>
              </div>
              {zipPath && <div className="selected-file"><span>▣</span> {zipPath}</div>}
              <div className="form-footer">
                <p><span className="hint-icon">i</span> Public GitHub repositories are supported in this milestone. Private repo authentication comes later.</p>
                <button className="button button-primary" type="submit" disabled={busy || !sourceInput.trim()}>{busy ? <><span className="spinner" /> Inspecting…</> : <>Inspect source <span>→</span></>}</button>
              </div>
            </form>
          </section>

          <section className="workflow-section">
            <div className="section-title-row"><div><div className="eyebrow">PIPELINE</div><h2>Build workflow</h2></div><span className={`status-pill ${busy ? "working" : detectedColor}`}><span />{phase}</span></div>
            <div className="phase-grid">
              {phases.map((item, index) => {
                const completed = Boolean(result) && index < 3;
                const active = busy && index < 3;
                return <div className={`phase-card ${completed ? "completed" : ""} ${active ? "current" : ""}`} key={item.title}>
                  <div className="phase-top"><span className="phase-index">{completed ? "✓" : `0${index + 1}`}</span>{index < 3 && <span className="phase-state">{completed ? "DONE" : active && index === 0 ? "RUNNING" : "WAITING"}</span>}{index === 3 && <span className="phase-state future">PLANNED</span>}</div>
                  <strong>{item.title}</strong><p>{item.detail}</p>
                  <div className="phase-bar"><span style={{ width: completed ? "100%" : active && index === 0 ? "48%" : "0%" }} /></div>
                </div>;
              })}
            </div>
          </section>

          {result && <section className="result-card panel">
            <div className="result-heading"><div><div className="eyebrow">INSPECTION RESULT</div><h2>{result.name}</h2></div><span className="project-type">{result.projectType}</span></div>
            <p className="result-path">{result.rootPath}</p>
            <div className="result-divider" />
            <div className="result-details"><strong>Detected markers</strong><div className="marker-list">{result.indicators.map((item) => <span key={item}>{item}</span>)}</div></div>
            <div className="recommendation"><span>↗</span><div><strong>Recommended next step</strong><p>{result.nextStep}</p></div></div>
          </section>}

          <section className="console-section">
            <div className="console-header"><div><span className="console-leds"><i /><i /><i /></span><strong>Process output</strong><span className="console-tag">LIVE LOG</span></div><button type="button" className="clear-button" onClick={() => setLogs([])}>Clear logs</button></div>
            <div className="terminal">
              <div className="terminal-meta"><span>REPOPACK / JOB CONSOLE</span><span>{logs.length.toString().padStart(3, "0")} ENTRIES</span></div>
              <div className="terminal-lines">{logs.map((line, index) => <div className="terminal-line" key={`${index}-${line}`}><span className="line-no">{String(index + 1).padStart(2, "0")}</span><code>{line}</code></div>)}<div className="terminal-cursor"><span className="line-no">›</span><span className="cursor-block" /></div><div ref={logEnd} /></div>
            </div>
          </section>

          <footer className="page-footer"><span>REPOPACK <span className="footer-dot">·</span> LOCAL BUILD STUDIO</span><span>Build commands will require your approval.</span></footer>
        </div>
      </main>
    </div>
  );
}
