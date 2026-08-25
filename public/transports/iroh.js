// The iroh transport: an iroh endpoint per browser, joined to a gossip topic.
//
// Browsers cannot send UDP, so every connection here is carried by a relay over
// WebSocket. The relay forwards encrypted QUIC and cannot read a vote, but it
// is on the path for the whole session.

import init, { Session, deck } from '../pkg/poker.js';

export default {
  id: 'iroh',
  label: 'iroh',
  tagline: 'QUIC over a relay',

  async load() {
    await init();
    return { deck: deck() };
  },

  async join({ invite, name, relay, onState, onStatus }) {
    const session = await Session.join(
      invite || undefined,
      name,
      relay || undefined,
      (json) => onState(JSON.parse(json)),
      onStatus,
    );

    return {
      id: session.endpointId,
      invite: session.invite,
      vote: (value) => session.vote(value),
      reveal: () => session.reveal(),
      reset: () => session.reset(),
      setName: (n) => session.setName(n),
      leave: () => session.leave(),

      // Nothing to measure: in a browser iroh is relay-only by construction.
      async paths() {
        return [{ kind: 'relay', label: 'every peer', detail: 'relayed (browsers cannot hole punch)' }];
      },
    };
  },
};
