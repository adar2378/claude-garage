import React, { useEffect, useRef } from "react";
import { useSettings } from "../lib/settings.js";
import { PET_FRAMES, PET_PROFILES, petMood } from "../lib/pet.js";

const TICK_MS = 300;

// The pit pet (lib/pet.js has the concept + frames). Rendered fixed just
// above the footer key strip; animation is driven imperatively (classList
// + textContent on a 300ms tick) rather than through React state — a pet
// re-rendering the app tree every 300ms would be a rude houseguest.
// Clicking it while a session is blocked is the same jump as `a`;
// clicking it otherwise is, of course, petting it.
export default function PitPet({ sessions, connState, onJump }) {
  const [settings] = useSettings();
  const kind = settings.pet ?? "off";

  const elRef = useRef(null);
  const artRef = useRef(null);
  // Live wall state for the tick without re-subscribing the interval.
  const dataRef = useRef({ sessions, connState });
  useEffect(() => {
    dataRef.current = { sessions, connState };
  }, [sessions, connState]);
  const onJumpRef = useRef(onJump);
  useEffect(() => {
    onJumpRef.current = onJump;
  }, [onJump]);

  const petRef = useRef({ x: 460, target: 460, frame: 0, tick: 0, celebrateUntil: 0, prevNeeds: 0 });

  useEffect(() => {
    if (kind === "off") return undefined;
    const profile = PET_PROFILES[kind];
    const frames = PET_FRAMES[kind];
    const p = petRef.current;

    function tick() {
      const el = elRef.current;
      const art = artRef.current;
      if (!el || !art) return;
      p.tick++;
      const { sessions: ss, connState: cs } = dataRef.current;
      const now = Date.now();
      const needs = ss.filter((s) => s.status === "needs-input").length;
      if (p.prevNeeds > 0 && needs === 0 && ss.length > 0) {
        p.celebrateUntil = now + profile.celebrateMs;
      }
      p.prevNeeds = needs;
      const mood = now < p.celebrateUntil ? "celebrate" : petMood({ sessions: ss, connState: cs });

      // movement, in character
      const maxX = Math.max(120, window.innerWidth - 160);
      if (mood === "alert") {
        p.target = 16;
      } else if (mood === "celebrate" && profile.zoomies) {
        if (p.tick % 3 === 0) p.target = p.target < maxX / 2 ? maxX - 40 : 40;
      } else if (mood !== "box" && mood !== "celebrate" && Math.random() < 0.05) {
        const stroll = 80 + Math.random() * (maxX - 80);
        if (Math.random() >= (profile.aloof || 0)) p.target = stroll;
      }
      const step = mood === "celebrate" && profile.zoomies ? profile.speed * 2 : profile.speed;
      p.x += Math.max(-step, Math.min(step, p.target - p.x));
      p.x = Math.max(8, Math.min(maxX, p.x));
      el.style.left = `${Math.round(p.x)}px`;
      const walking = Math.abs(p.target - p.x) > 6;

      // frames at the species' own cadence
      if (p.tick % profile.frameEvery === 0) p.frame++;
      const moodFrames = frames[mood] ?? frames.watch;
      art.textContent = moodFrames[p.frame % moodFrames.length].join("\n");

      el.classList.toggle("alert", mood === "alert");
      el.classList.toggle("deadpan", !!profile.deadpan);
      el.classList.toggle("waddle", !!profile.waddle && walking);
      el.classList.toggle(
        "bounce-big",
        profile.alertBounce === "bounce-big" && (mood === "alert" || (mood === "celebrate" && profile.zoomies))
      );
      el.classList.toggle("bounce-small", profile.alertBounce === "bounce-small" && mood === "alert");
      el.title =
        mood === "alert"
          ? `${needs} session${needs === 1 ? "" : "s"} need${needs === 1 ? "s" : ""} input — click to jump`
          : `pit pet — ${
              mood === "box" ? "hiding until the daemon is back" : mood === "celebrate" ? "all clear!" : `${mood}ing`
            }`;
    }

    const interval = setInterval(tick, TICK_MS);
    tick();
    return () => clearInterval(interval);
  }, [kind]);

  if (kind === "off") return null;

  return (
    <div
      ref={elRef}
      role="button"
      aria-label="pit pet — click to jump to a session that needs input"
      className="pit-pet"
      onClick={() => {
        const { sessions: ss, connState: cs } = dataRef.current;
        if (petMood({ sessions: ss, connState: cs }) === "alert") {
          onJumpRef.current?.();
        } else {
          // petting it is allowed
          petRef.current.celebrateUntil = Date.now() + 1200;
        }
      }}
    >
      <span className="pit-pet-bubble">!</span>
      <pre ref={artRef} />
    </div>
  );
}
