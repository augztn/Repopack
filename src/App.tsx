import { useEffect, useRef, useState, type FormEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";

type LogEvent = { level: string; message: string };
type ToolCheck = { tool: string; available: boolean; version?: string; message: string };
type BuildArtifact = { name: string; path: string; sizeBytes: number; kind: string };
type BuildResult = { profile: string; exitCode?: number; success: boolean; message: string; artifacts: BuildArtifact[] };
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
  { title: "Build", detail: "Approved local build profiles" },
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
  const [toolChecks, setToolChecks] = useState<ToolCheck[] | null>(null);
  const [checkingTools, setCheckingTools] = useState(false);
  const [buildProfile, setBuildProfile] = useState("node-build");
  const [building, setBuilding] = useState(false);
  const [buildResult, setBuildResult] = useState<BuildResult | null>(null);
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
      setBuildResult(null);
      setBuildProfile(prepared.projectType === "tauri" ? "tauri-build" : prepared.projectType === "electron" ? "electron-build" : prepared.projectType === "flutter" ? "flutter-apk" : prepared.projectType === "android-gradle" ? "android-debug-apk" : "node-build");
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

  async function checkTools() {
    setCheckingTools(true);
    try {
      const checks = await invoke<ToolCheck[]>("check_build_tools");
      setToolChecks(checks);
      setLogs((current) => [...current, `[info] Checked ${checks.length} local build tools.`]);
    } catch (error) {
      setLogs((current) => [...current, `[error] Toolchain check failed: ${String(error)}`]);
    } finally {
      setCheckingTools(false);
    }
  }

  async function runBuild() {
    if (!result) return;
    const labels: Record<string, string> = {
      "tauri-build": "npm run tauri build",
      "node-build": "npm run build",
      "electron-build": "npm run build (Electron project)",
      "flutter-windows": "flutter build windows",
      "flutter-linux": "flutter build linux",
      "flutter-apk": "flutter build apk",
      "android-debug-apk": "Gradle assembleDebug",
    };
    const command = labels[buildProfile] ?? buildProfile;
    if (!window.confirm(`Run ${command} in this project? Build scripts can execute arbitrary code. Continue only if you trust this repository.`)) return;
    setBuilding(true);
    setBuildResult(null);
    setPhase("Build running");
    setLogs((current) => [...current, `[approval] User approved build profile: ${buildProfile}`]);
    try {
      const outcome = await invoke<BuildResult>("run_approved_build", { rootPath: result.rootPath, profile: buildProfile });
      setBuildResult(outcome);
      setPhase(outcome.success ? "Build complete" : "Build failed");
    } catch (error) {
      setPhase("Build failed");
      setLogs((current) => [...current, `[error] ${String(error)}`]);
    } finally {
      setBuilding(false);
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
            <div className="section-title-row"><div><div className="eyebrow">PIPELINE</div><h2>Build workflow</h2></div><span className={`status-pill ${busy || building ? "working" : detectedColor}`}><span />{phase}</span></div>
            <div className="phase-grid">
              {phases.map((item, index) => {
                const completed = (Boolean(result) && index < 3) || (Boolean(buildResult?.success) && index === 3);
                const active = (busy && index < 3) || (building && index === 3);
                return <div className={`phase-card ${completed ? "completed" : ""} ${active ? "current" : ""}`} key={item.title}>
                  <div className="phase-top"><span className="phase-index">{completed ? "✓" : `0${index + 1}`}</span>{index < 3 && <span className="phase-state">{completed ? "DONE" : active && index === 0 ? "RUNNING" : "WAITING"}</span>}{index === 3 && <span className={`phase-state ${completed ? "" : active ? "" : "future"}`}>{completed ? "DONE" : active ? "RUNNING" : "READY"}</span>}</div>
                  <strong>{item.title}</strong><p>{item.detail}</p>
                  <div className="phase-bar"><span style={{ width: completed ? "100%" : active ? "48%" : "0%" }} /></div>
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

          {result && <section className="result-card panel build-controls">
            <div className="result-heading"><div><div className="eyebrow">LOCAL TOOLCHAIN</div><h2>Build this project</h2></div><button type="button" className="button button-secondary" onClick={checkTools} disabled={checkingTools}>{checkingTools ? "Checking…" : "Check tools"}</button></div>
            <p className="result-path">Builds run locally in the inspected workspace. Review the profile and approve execution explicitly.</p>
            <div className="build-action-row">
              <label className="build-profile-label" htmlFor="build-profile">BUILD PROFILE</label>
              <select id="build-profile" value={buildProfile} onChange={(event) => { setBuildProfile(event.target.value); setBuildResult(null); }} disabled={building}>
                <option value="tauri-build">Tauri desktop package</option>
                <option value="node-build">Node / web build</option>
                <option value="electron-build">Electron project build script</option>
                <option value="flutter-windows">Flutter — Windows</option>
                <option value="flutter-linux">Flutter — Linux</option>
                <option value="flutter-apk">Flutter — Android APK</option>
                <option value="android-debug-apk">Android Gradle — debug APK</option>
              </select>
              <button type="button" className="button button-primary" onClick={runBuild} disabled={building}>{building ? <><span className="spinner" /> Building…</> : <>Approve &amp; build <span>→</span></>}</button>
            </div>
            {toolChecks && <div className="tool-check-grid">{toolChecks.map((tool) => <div className={`tool-check ${tool.available ? "available" : "missing"}`} key={tool.tool}><span>{tool.available ? "✓" : "×"}</span><div><strong>{tool.tool}</strong><small>{tool.version ?? tool.message}</small></div></div>)}</div>}
            {buildResult && <div className={`build-result ${buildResult.success ? "available" : "missing"}`}><strong>{buildResult.success ? "Build completed" : "Build failed"}</strong><p>{buildResult.message}</p></div>}
            {buildResult?.artifacts?.length > 0 && <div className="artifact-panel">
              <div className="artifact-heading"><strong>Detected output artifacts</strong><span>{buildResult.artifacts.length} FILE{buildResult.artifacts.length === 1 ? "" : "S"}</span></div>
              {buildResult.artifacts.map((artifact) => <div className="artifact-row" key={artifact.path}>
                <div className="artifact-file-icon">{artifact.kind}</div>
                <div className="artifact-meta"><strong>{artifact.name}</strong><small>{artifact.path}</small><small>{artifact.sizeBytes < 1024 * 1024 ? `${(artifact.sizeBytes / 1024).toFixed(1)} KB` : `${(artifact.sizeBytes / (1024 * 1024)).toFixed(2)} MB`}</small></div>
                <button type="button" className="button button-secondary" onClick={async () => {
                  try { await invoke("open_artifact_folder", { artifactPath: artifact.path }); }
                  catch (error) { setLogs((current) => [...current, `[error] Could not open artifact folder: ${String(error)}`]); }
                }}>Open folder ↗</button>
              </div>)}
            </div>}
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
