// The wireless dialog listens for phone requests while it is open and leaves the listener as it found it.
export const createDialogOfferListener = () => {
  let owned: string | null = null;
  let isOpen = true;
  return {
    open() { isOpen = true; },
    async ensure(key: string, running: boolean, start: () => Promise<boolean>, stop: () => void) {
      if (running && (owned === null || owned === key)) return;
      if (!await start()) return;
      if (isOpen) owned = key; else stop();
    },
    release(stop: () => void) {
      isOpen = false;
      if (owned === null) return;
      owned = null;
      stop();
    },
  };
};
