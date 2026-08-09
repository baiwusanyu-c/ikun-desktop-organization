// app.ts - Application state store using Svelte 5 $state runes

let windowTitle = $state('Desktop Organizer');
let isDevMode = $state(false);
let saveStatus = $state<'idle' | 'saving' | 'saved' | 'error'>('idle');

export function getWindowTitle() { return windowTitle; }
export function setWindowTitle(title: string) { windowTitle = title; }
export function getIsDevMode() { return isDevMode; }
export function setIsDevMode(v: boolean) { isDevMode = v; }
export function getSaveStatus() { return saveStatus; }
export function setSaveStatus(s: typeof saveStatus) { saveStatus = s; }
