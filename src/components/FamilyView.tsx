// The family: me (name, look, theme), everyone else, and for parents, adding people and devices.

import { ArrowUp, LogOut, Moon, Plus, Power, QrCode, Server, Smartphone, Sun, Trash2, X } from "lucide-react";
import { useEffect, useState } from "react";
import { api, errorText, isDesktopApp } from "../lib/api";
import { ago } from "../lib/format";
import { useAppState } from "../lib/state";
import { COLORS, type Device, type JoinCode, type Member, type Role } from "../lib/types";
import { Kit } from "./Kit";
import { AVATARS, Avatar, Button, Field, inputClass, Sheet, toast } from "./ui";

const THEME_KEY = "kitchat.theme";

export function storedTheme(): "day" | "bedtime" {
  try {
    return localStorage.getItem(THEME_KEY) === "bedtime" ? "bedtime" : "day";
  } catch {
    return "day";
  }
}

export function applyTheme(theme: "day" | "bedtime") {
  if (theme === "bedtime") document.documentElement.dataset.theme = "bedtime";
  else delete document.documentElement.dataset.theme;
  try {
    localStorage.setItem(THEME_KEY, theme);
  } catch {
    // Not saved; it still applies until the app closes.
  }
}

function LookPicker({
  avatar,
  color,
  onAvatar,
  onColor,
}: {
  avatar: string;
  color: string;
  onAvatar: (a: string) => void;
  onColor: (c: string) => void;
}) {
  return (
    <div>
      <div className="flex flex-wrap gap-1">
        {AVATARS.map((a) => (
          <button
            key={a}
            onClick={() => onAvatar(a)}
            className={`grid size-12 place-items-center rounded-xl text-2xl ${avatar === a ? "bg-brand-soft ring-2 ring-brand" : "hover:bg-surface-2"}`}
          >
            {a}
          </button>
        ))}
      </div>
      <div className="mt-3 flex gap-2">
        {COLORS.map((c) => (
          <button
            key={c}
            aria-label={c}
            onClick={() => onColor(c)}
            className={`tint-${c} size-10 rounded-full ${color === c ? "ring-4 ring-ink/30" : ""}`}
            style={{ background: "var(--tint)" }}
          />
        ))}
      </div>
    </div>
  );
}

function EditMe({ me, onClose }: { me: Member; onClose: () => void }) {
  const [name, setName] = useState(me.name);
  const [avatar, setAvatar] = useState(me.avatar);
  const [color, setColor] = useState(me.color);
  const save = async () => {
    try {
      await api.updateMember(me.id, { name: name.trim() || me.name, avatar, color });
      onClose();
    } catch (e) {
      toast(errorText(e), "error");
    }
  };
  return (
    <Sheet title="My look" onClose={onClose}>
      <div className="flex flex-col gap-4">
        <div className="flex justify-center">
          <Avatar member={{ ...me, avatar, color }} size={88} ring />
        </div>
        <Field label="Name">
          <input className={inputClass} value={name} maxLength={40} onChange={(e) => setName(e.target.value)} />
        </Field>
        <LookPicker avatar={avatar} color={color} onAvatar={setAvatar} onColor={setColor} />
        <Button big onClick={() => void save()}>
          Save
        </Button>
      </div>
    </Sheet>
  );
}

function AddMember({ onClose, onAdded }: { onClose: () => void; onAdded: (m: Member) => void }) {
  const [name, setName] = useState("");
  const [role, setRole] = useState<Role>("kid");
  const [avatar, setAvatar] = useState(AVATARS[1]);
  const [color, setColor] = useState<string>("sky");
  const add = async () => {
    if (!name.trim()) return;
    try {
      const member = await api.addMember({ name: name.trim(), role, avatar, color });
      onAdded(member);
    } catch (e) {
      toast(errorText(e), "error");
    }
  };
  return (
    <Sheet title="Add someone to the family" onClose={onClose}>
      <div className="flex flex-col gap-4">
        <Field label="Name">
          <input
            className={inputClass}
            value={name}
            maxLength={40}
            autoFocus
            onChange={(e) => setName(e.target.value)}
          />
        </Field>
        <div className="grid grid-cols-2 gap-2">
          {(["kid", "parent"] as const).map((r) => (
            <button
              key={r}
              onClick={() => setRole(r)}
              className={`rounded-2xl border-2 p-3 font-extrabold ${role === r ? "border-brand bg-brand-soft" : "border-line"}`}
            >
              {r === "kid" ? "🧒 Kid" : "🧑 Parent"}
            </button>
          ))}
        </div>
        <LookPicker avatar={avatar} color={color} onAvatar={setAvatar} onColor={setColor} />
        <Button big disabled={!name.trim()} onClick={() => void add()}>
          <Plus /> Add
        </Button>
      </div>
    </Sheet>
  );
}

function Countdown({ until }: { until: number }) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const t = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(t);
  }, []);
  const left = Math.max(0, Math.round((until - now) / 1000));
  if (left === 0) return <span className="text-danger">This code has run out. Make a new one.</span>;
  return (
    <span>
      Works for {Math.floor(left / 60)}:{String(left % 60).padStart(2, "0")} more
    </span>
  );
}

function JoinCodeSheet({ member, onClose }: { member: Member; onClose: () => void }) {
  const [code, setCode] = useState<JoinCode | null>(null);
  const make = async () => {
    try {
      setCode(await api.joinCode(member.id));
    } catch (e) {
      toast(errorText(e), "error");
      onClose();
    }
  };
  useEffect(() => {
    void make();
    // Once, when opened.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return (
    <Sheet title={`Add a device for ${member.name}`} onClose={onClose}>
      {!code ? (
        <div className="py-10 text-center text-ink-soft">Making a code…</div>
      ) : (
        <div className="flex flex-col items-center gap-3 text-center">
          <p className="text-ink-soft">
            On {member.name}'s tablet or phone, open KitChat, tap <b>Join my family</b> and scan this, or type the code.
          </p>
          <div
            className="w-60 rounded-2xl bg-white p-3 [&_svg]:h-auto [&_svg]:w-full"
            dangerouslySetInnerHTML={{ __html: code.qrSvg }}
          />
          <div className="font-mono text-4xl font-black tracking-[0.3em]">{code.code}</div>
          <div className="text-sm text-ink-soft">
            Hub:{" "}
            {code.hubUrls.map((url) => (
              <span key={url} className="block font-mono break-all">
                {url}
              </span>
            ))}
          </div>
          <div className="text-sm font-bold text-ink-soft">
            <Countdown until={code.expiresAt} />
          </div>
          <Button tone="soft" onClick={() => void make()}>
            New code
          </Button>
        </div>
      )}
    </Sheet>
  );
}

function Devices({ members }: { members: Member[] }) {
  const [devices, setDevices] = useState<Device[] | null>(null);
  const mine = useAppState((s) => s.account?.deviceId);
  const load = () =>
    api
      .devices()
      .then(setDevices)
      .catch((e) => toast(errorText(e), "error"));
  useEffect(() => {
    void load();
  }, []);
  const revoke = async (d: Device) => {
    const who = members.find((m) => m.id === d.memberId)?.name ?? "someone";
    if (!window.confirm(`Remove "${d.name}" (${who})? It will be signed out and need a new code to come back.`)) return;
    try {
      await api.revokeDevice(d.id);
      await load();
    } catch (e) {
      toast(errorText(e), "error");
    }
  };
  if (!devices) return null;
  return (
    <div className="card divide-y-2 divide-line">
      {devices.map((d) => {
        const owner = members.find((m) => m.id === d.memberId);
        return (
          <div key={d.id} className="flex items-center gap-3 p-3">
            <Smartphone className="shrink-0 text-ink-soft" />
            <div className="min-w-0 flex-1">
              <div className="truncate font-extrabold">
                {d.name} {d.id === mine && <span className="text-sm text-ink-soft">(this one)</span>}
              </div>
              <div className="text-sm text-ink-soft">
                {owner?.name ?? "?"} · last seen {ago(d.lastSeen)}
              </div>
            </div>
            {d.id !== mine && (
              <button
                aria-label={`Remove ${d.name}`}
                onClick={() => void revoke(d)}
                className="grid size-11 place-items-center rounded-full text-danger hover:bg-surface-2"
              >
                <Trash2 size={20} />
              </button>
            )}
          </div>
        );
      })}
    </div>
  );
}

const MAX_HUB_ADDRESSES = 5;

/** The addresses this device tries for the family's hub, in order: say Tailscale first, then the home network. */
function HubAddressesSheet({ current, inUse, onClose }: { current: string[]; inUse: string; onClose: () => void }) {
  const [urls, setUrls] = useState(current);
  const [busy, setBusy] = useState(false);
  const filled = urls.map((url) => url.trim()).filter(Boolean);
  const changed = filled.join("\n") !== current.join("\n");

  const setUrl = (index: number, url: string) => setUrls(urls.map((u, i) => (i === index ? url : u)));
  const moveUp = (index: number) => {
    const next = [...urls];
    [next[index - 1], next[index]] = [next[index], next[index - 1]];
    setUrls(next);
  };

  const save = async () => {
    setBusy(true);
    try {
      const saved = await api.setHubAddresses(filled);
      toast(
        saved.unreachable.length > 0
          ? `Saved. Couldn't reach ${saved.unreachable.join(", ")} from here right now.`
          : "Hub addresses saved",
      );
      onClose();
    } catch (e) {
      toast(errorText(e), "error");
    } finally {
      setBusy(false);
    }
  };

  return (
    <Sheet title="Family hub addresses" onClose={onClose}>
      <div className="flex flex-col gap-4">
        <p className="text-ink-soft">
          KitChat tries these from the top and uses the first that works, so the family can keep chatting at home when
          the internet is down. This device stays signed in.
        </p>
        <div className="flex flex-col gap-2">
          {urls.map((url, i) => (
            <div key={i} className="flex items-center gap-1">
              <input
                aria-label={`Hub address ${i + 1}`}
                className={`${inputClass} min-w-0 flex-1 px-3! font-mono text-[13px]!`}
                value={url}
                spellCheck={false}
                autoCapitalize="off"
                autoCorrect="off"
                inputMode="url"
                placeholder={i === 0 ? "100.101.102.103:8787" : "192.168.1.20:8787"}
                onChange={(e) => setUrl(i, e.target.value)}
              />
              <button
                type="button"
                aria-label={`Try address ${i + 1} sooner`}
                disabled={i === 0}
                onClick={() => moveUp(i)}
                className="grid size-10 shrink-0 place-items-center rounded-full hover:bg-brand-soft disabled:opacity-30"
              >
                <ArrowUp size={20} />
              </button>
              <button
                type="button"
                aria-label={`Remove address ${i + 1}`}
                disabled={urls.length === 1}
                onClick={() => setUrls(urls.filter((_, j) => j !== i))}
                className="grid size-10 shrink-0 place-items-center rounded-full text-danger hover:bg-surface-2 disabled:opacity-30"
              >
                <X size={20} />
              </button>
            </div>
          ))}
        </div>
        <p className="text-sm break-all text-ink-soft">
          In use now: <span className="font-mono">{inUse}</span>
        </p>
        {urls.length < MAX_HUB_ADDRESSES && (
          <Button tone="soft" onClick={() => setUrls([...urls, ""])}>
            <Plus /> Add another address
          </Button>
        )}
        <Button big disabled={busy || filled.length === 0 || !changed} onClick={() => void save()}>
          {busy ? "Checking…" : "Save addresses"}
        </Button>
      </div>
    </Sheet>
  );
}

/** Desktop: start KitChat (in the tray) when signing in to the computer. */
function Autostart() {
  const [on, setOn] = useState<boolean | null>(null);
  useEffect(() => {
    void api.autostartEnabled().then(setOn, () => setOn(false));
  }, []);
  if (on === null) return null;
  return (
    <button
      role="switch"
      aria-checked={on}
      onClick={() => void api.setAutostart(!on).then(setOn, (e) => toast(errorText(e), "error"))}
      className="card flex w-full items-center gap-3 p-4 text-left font-extrabold"
    >
      <Power />
      <span className="flex-1">
        Start KitChat when I sign in
        <span className="block text-sm font-normal text-ink-soft">It waits in the tray, so messages still arrive.</span>
      </span>
      <span className={`relative h-7 w-12 rounded-full transition ${on ? "bg-brand" : "bg-line"}`}>
        <span className={`absolute top-1 size-5 rounded-full bg-white transition-all ${on ? "left-6" : "left-1"}`} />
      </span>
    </button>
  );
}

export function FamilyView({ onOpenPerson }: { onOpenPerson: (memberId: string) => void }) {
  const account = useAppState((s) => s.account)!;
  const members = useAppState((s) => s.members);
  const status = useAppState((s) => s.status);
  const me = members.find((m) => m.id === account.me.id) ?? account.me;
  const parent = me.role === "parent";
  const [theme, setTheme] = useState(storedTheme());
  const [editing, setEditing] = useState(false);
  const [adding, setAdding] = useState(false);
  const [codeFor, setCodeFor] = useState<Member | null>(null);
  const [changingHub, setChangingHub] = useState(false);

  const others = members.filter((m) => m.id !== me.id);

  const signOut = async () => {
    if (!window.confirm("Sign this device out? You'll need a new code from a parent to sign back in.")) return;
    try {
      await api.signOut();
    } catch (e) {
      toast(errorText(e), "error");
    }
  };

  return (
    <div className="mx-auto flex max-w-2xl flex-col gap-6 p-4">
      <div className="flex items-center gap-3">
        <Kit mood="hug" size={64} />
        <div>
          <h2 className="text-2xl font-black">{account.familyName}</h2>
          <p className="text-ink-soft">
            {status.state === "online" ? "Connected to the family hub" : "Not connected right now"}
          </p>
        </div>
      </div>

      <section>
        <h3 className="mb-2 px-1 text-sm font-black tracking-wide text-ink-soft uppercase">Me</h3>
        <div className="card flex items-center gap-4 p-4">
          <Avatar member={me} size={64} ring />
          <div className="min-w-0 flex-1">
            <div className="truncate text-xl font-black">{me.name}</div>
            <div className="text-ink-soft">{parent ? "Parent" : "Kid"}</div>
          </div>
          <Button tone="soft" onClick={() => setEditing(true)}>
            Change
          </Button>
        </div>
        <button
          onClick={() => {
            const next = theme === "day" ? "bedtime" : "day";
            applyTheme(next);
            setTheme(next);
          }}
          className="card mt-2 flex w-full items-center gap-3 p-4 text-left font-extrabold"
        >
          {theme === "day" ? <Moon /> : <Sun />}
          {theme === "day" ? "Switch to bedtime colours" : "Switch to daytime colours"}
        </button>
      </section>

      <section>
        <h3 className="mb-2 px-1 text-sm font-black tracking-wide text-ink-soft uppercase">Family</h3>
        <div className="card divide-y-2 divide-line">
          {others.map((m) => (
            <div key={m.id} className="flex items-center gap-3 p-3">
              <button className="flex min-w-0 flex-1 items-center gap-3 text-left" onClick={() => onOpenPerson(m.id)}>
                <Avatar member={m} size={48} />
                <div className="min-w-0">
                  <div className="truncate text-lg font-extrabold">{m.name}</div>
                  <div className="text-sm text-ink-soft">{m.role === "parent" ? "Parent" : "Kid"} · tap to chat</div>
                </div>
              </button>
              {parent && (
                <button
                  aria-label={`Add a device for ${m.name}`}
                  title="Add a device"
                  onClick={() => setCodeFor(m)}
                  className="grid size-11 place-items-center rounded-full hover:bg-brand-soft"
                >
                  <QrCode size={22} />
                </button>
              )}
            </div>
          ))}
          {others.length === 0 && <div className="p-4 text-ink-soft">Nobody else yet.</div>}
        </div>
      </section>

      {parent && (
        <section>
          <h3 className="mb-2 px-1 text-sm font-black tracking-wide text-ink-soft uppercase">Parent tools</h3>
          <div className="flex flex-wrap gap-2">
            <Button onClick={() => setAdding(true)}>
              <Plus /> Add someone
            </Button>
            <Button tone="soft" onClick={() => setCodeFor(me)}>
              <QrCode /> Add another device for me
            </Button>
          </div>
          <h4 className="mt-4 mb-2 px-1 font-extrabold">Devices</h4>
          <Devices members={members} />
        </section>
      )}

      <section>
        <h3 className="mb-2 px-1 text-sm font-black tracking-wide text-ink-soft uppercase">This device</h3>
        <div className="flex flex-col gap-2">
          {isDesktopApp && <Autostart />}
          <div className="card flex items-center gap-3 p-4">
            <Server />
            <div className="min-w-0 flex-1">
              <div className="font-extrabold">Family hub</div>
              <div className="truncate font-mono text-sm text-ink-soft">{account.hubUrl}</div>
              {account.hubUrls.length > 1 && (
                <div className="text-sm text-ink-soft">{account.hubUrls.length} addresses, tried in order</div>
              )}
            </div>
            <Button tone="soft" onClick={() => setChangingHub(true)}>
              Change
            </Button>
          </div>
        </div>
      </section>

      <Button tone="plain" className="self-start text-ink-soft" onClick={() => void signOut()}>
        <LogOut /> Sign out of this device
      </Button>

      {editing && <EditMe me={me} onClose={() => setEditing(false)} />}
      {adding && (
        <AddMember
          onClose={() => setAdding(false)}
          onAdded={(m) => {
            setAdding(false);
            toast(`${m.name} is in the family! Now add their device.`);
            setCodeFor(m);
          }}
        />
      )}
      {codeFor && <JoinCodeSheet member={codeFor} onClose={() => setCodeFor(null)} />}
      {changingHub && (
        <HubAddressesSheet current={account.hubUrls} inUse={account.hubUrl} onClose={() => setChangingHub(false)} />
      )}
    </div>
  );
}
