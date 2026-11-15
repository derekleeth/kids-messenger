// The first screen: join the family with a code a parent made, or (for the first parent) set the family up.

import { ArrowLeft, QrCode, Sparkles, Users } from "lucide-react";
import { useEffect, useState } from "react";
import { api, errorText, isTauri } from "../lib/api";
import { parseJoinLink } from "../lib/links";
import { useAppState } from "../lib/state";
import { Kit } from "./Kit";
import { Button, Field, inputClass } from "./ui";

type Mode = "start" | "join" | "setup";

export function Welcome() {
  const [mode, setMode] = useState<Mode>("start");
  const why = useAppState((s) => s.status.detail);
  const [deviceName, setDeviceName] = useState("");
  useEffect(() => {
    api.deviceNameHint().then(setDeviceName, () => {});
  }, []);

  return (
    <div className="safe-top safe-bottom scroll-area flex h-full flex-col items-center bg-[radial-gradient(circle_at_50%_0%,var(--sky-soft),var(--bg)_60%)] px-5 py-8">
      <div className="flex w-full max-w-md flex-col items-center">
        <Kit mood={mode === "setup" ? "party" : "wave"} size={170} className="anim-breathe" />
        <h1 className="mt-2 text-center text-4xl font-black tracking-tight">KitChat</h1>
        <p className="mb-6 text-center text-lg text-ink-soft">Chat, share and make lists with your family.</p>
        {why && mode === "start" && (
          <div className="mb-4 w-full rounded-2xl bg-[var(--sun-soft)] p-4 text-center font-bold">{why}</div>
        )}
        {mode === "start" && (
          <div className="flex w-full flex-col gap-3">
            <Button big onClick={() => setMode("join")}>
              <Users /> Join my family
            </Button>
            <Button big tone="soft" onClick={() => setMode("setup")}>
              <Sparkles /> Set up a new family
            </Button>
            <p className="mt-2 text-center text-sm text-ink-soft">
              Setting up is for a parent, once. Everyone else joins with a code from a parent.
            </p>
          </div>
        )}
        {mode === "join" && <JoinForm deviceName={deviceName} onBack={() => setMode("start")} />}
        {mode === "setup" && <SetupForm deviceName={deviceName} onBack={() => setMode("start")} />}
      </div>
    </div>
  );
}

function JoinForm({ deviceName, onBack }: { deviceName: string; onBack: () => void }) {
  const [hub, setHub] = useState("");
  const [code, setCode] = useState("");
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const device = name || deviceName;

  const takeLink = (text: string) => {
    const link = parseJoinLink(text);
    if (link) {
      setHub(link.hub);
      setCode(link.code);
      return true;
    }
    return false;
  };

  const scan = async () => {
    setError(null);
    try {
      const text = await api.scanQr();
      if (text && !takeLink(text)) setError("That QR code isn't a KitChat join code.");
    } catch (e) {
      setError(errorText(e));
    }
  };

  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.hubHello(hub);
      await api.joinFamily({ url: hub, code, deviceName: device || "Tablet" });
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <form
      className="card flex w-full flex-col gap-4 p-5"
      onSubmit={(e) => {
        e.preventDefault();
        void submit();
      }}
    >
      <div className="flex items-center gap-2">
        <button type="button" onClick={onBack} className="rounded-full p-2 hover:bg-brand-soft" aria-label="Back">
          <ArrowLeft />
        </button>
        <h2 className="text-2xl font-black">Join my family</h2>
      </div>
      {isTauri && (
        <Button tone="soft" onClick={scan}>
          <QrCode /> Scan the code on a parent's screen
        </Button>
      )}
      <Field label="Family hub address" hint="A parent can tell you this. It looks like 192.168.1.20:8787.">
        <input
          className={inputClass}
          value={hub}
          onChange={(e) => !takeLink(e.target.value) && setHub(e.target.value)}
          placeholder="192.168.1.20:8787"
          autoCapitalize="off"
          autoCorrect="off"
          inputMode="url"
        />
      </Field>
      <Field label="Join code">
        <input
          className={`${inputClass} text-center font-mono text-2xl tracking-[0.3em] uppercase`}
          value={code}
          onChange={(e) => !takeLink(e.target.value) && setCode(e.target.value.toUpperCase())}
          placeholder="ABC123"
          maxLength={12}
          autoCapitalize="characters"
          autoCorrect="off"
        />
      </Field>
      <Field label="Name for this device">
        <input
          className={inputClass}
          value={device}
          onChange={(e) => setName(e.target.value)}
          placeholder="Sam's tablet"
        />
      </Field>
      {error && <p className="rounded-2xl bg-[var(--bubblegum-soft)] p-3 font-bold">{error}</p>}
      <Button big type="submit" disabled={busy || !hub.trim() || code.trim().length < 6}>
        {busy ? "Joining…" : "Join!"}
      </Button>
    </form>
  );
}

function SetupForm({ deviceName, onBack }: { deviceName: string; onBack: () => void }) {
  const [hub, setHub] = useState("localhost:8787");
  const [setupCode, setSetupCode] = useState("");
  const [familyName, setFamilyName] = useState("");
  const [name, setName] = useState("");
  const [device, setDevice] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      const hello = await api.hubHello(hub);
      if (!hello.needsSetup) {
        throw `The hub is already set up for "${hello.familyName}". Ask a parent there for a join code.`;
      }
      await api.setupFamily({ url: hub, setupCode, familyName, name, deviceName: device || deviceName || "Computer" });
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <form
      className="card flex w-full flex-col gap-4 p-5"
      onSubmit={(e) => {
        e.preventDefault();
        void submit();
      }}
    >
      <div className="flex items-center gap-2">
        <button type="button" onClick={onBack} className="rounded-full p-2 hover:bg-brand-soft" aria-label="Back">
          <ArrowLeft />
        </button>
        <h2 className="text-2xl font-black">Set up a new family</h2>
      </div>
      <p className="text-ink-soft">
        Start the family hub first (see <b>docs/hub_setup.md</b>). It shows a setup code when it starts.
      </p>
      <Field label="Family hub address">
        <input className={inputClass} value={hub} onChange={(e) => setHub(e.target.value)} autoCapitalize="off" />
      </Field>
      <Field label="Setup code">
        <input
          className={`${inputClass} text-center font-mono text-2xl tracking-[0.25em] uppercase`}
          value={setupCode}
          onChange={(e) => setSetupCode(e.target.value.toUpperCase())}
          maxLength={12}
          autoCapitalize="characters"
        />
      </Field>
      <Field label="Family name">
        <input
          className={inputClass}
          value={familyName}
          onChange={(e) => setFamilyName(e.target.value)}
          placeholder="The Lynx Family"
        />
      </Field>
      <Field label="Your name">
        <input className={inputClass} value={name} onChange={(e) => setName(e.target.value)} placeholder="Dad" />
      </Field>
      <Field label="Name for this device">
        <input
          className={inputClass}
          value={device || deviceName}
          onChange={(e) => setDevice(e.target.value)}
          placeholder="Desktop"
        />
      </Field>
      {error && <p className="rounded-2xl bg-[var(--bubblegum-soft)] p-3 font-bold">{error}</p>}
      <Button big type="submit" disabled={busy || !setupCode.trim() || !familyName.trim() || !name.trim()}>
        {busy ? "Setting up…" : "Set up my family"}
      </Button>
    </form>
  );
}
