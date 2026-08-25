// The WebRTC transport: Trystero for matchmaking, then direct DataChannels.
//
// Peers find each other over public Nostr relays (or a relay of your own with
// `?signal=`), exchange the WebRTC handshake there, and then talk directly.
// Once connected the signaling relay is out of the path entirely — which is the
// difference from the iroh transport, where the relay stays on it.
//
// Where a NAT refuses to be traversed, WebRTC falls back to a TURN server, so
// "direct" is a best effort rather than a guarantee. `paths()` reports what
// each connection actually settled on.

import init, { RoomHandle, deck, heartbeatMs } from '../pkg-webrtc/room.js';

const APP_ID = 'poker-planning';

// Trystero namespaces rooms by app id, so a room name only has to be
// unguessable, not globally unique.
function newRoomId() {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  return [...bytes].map((b) => b.toString(36).padStart(2, '0')).join('');
}

async function loadSignaling(signal) {
  if (signal) {
    const { joinRoom } = await import('../vendor/trystero-ws.js');
    return { joinRoom, config: { appId: APP_ID, relayConfig: { urls: [signal] } } };
  }
  const { joinRoom } = await import('../vendor/trystero-nostr.js');
  return { joinRoom, config: { appId: APP_ID } };
}

// Reads what a connection actually settled on: a direct path, or a TURN relay.
async function describeConnection(pc) {
  let pair = null;
  const stats = await pc.getStats();
  stats.forEach((report) => {
    if (report.type === 'candidate-pair' && report.state === 'succeeded' && report.nominated !== false) {
      pair = report;
    }
  });
  if (!pair) return { kind: 'pending', detail: 'still negotiating' };

  const local = stats.get(pair.localCandidateId);
  const remote = stats.get(pair.remoteCandidateId);
  const relayed = local?.candidateType === 'relay' || remote?.candidateType === 'relay';
  return {
    kind: relayed ? 'relay' : 'direct',
    detail: `${local?.candidateType ?? '?'} ↔ ${remote?.candidateType ?? '?'}`,
  };
}

export default {
  id: 'webrtc',
  label: 'WebRTC',
  tagline: 'direct, TURN as fallback',

  async load() {
    await init();
    return { deck: deck() };
  },

  async join({ invite, name, signal, turn, onState, onStatus }) {
    const roomId = invite || newRoomId();
    const room = new RoomHandle(name);

    onStatus('connecting', signal ? 'Connecting to signaling relay' : 'Finding peers');

    const { joinRoom, config } = await loadSignaling(signal);
    if (turn) {
      // `turnConfig` is merged into the ICE servers, so a room can be given a
      // fallback for the peers whose NAT will not be traversed.
      config.turnConfig = [turn];
    }

    const swarm = joinRoom(config, roomId);
    const state = swarm.makeAction('state');

    // Trystero's peer ids are its own; the room is keyed by the ed25519 key
    // that signs announcements. Keep the mapping so a peer leaving can be
    // matched to a participant.
    const authors = new Map();

    const render = () => onState(JSON.parse(room.view()));
    const announce = (target) => state.send(room.announce(), target ? { target } : undefined);

    state.onMessage = (data, { peerId }) => {
      const applied = room.receive(data instanceof Uint8Array ? data : new Uint8Array(data));
      if (applied.author) authors.set(peerId, applied.author);
      if (applied.changed) render();
      // Answer a reveal or a new round now rather than on the next heartbeat.
      if (applied.roundChanged) announce();
    };

    swarm.onPeerJoin = (peerId) => {
      onStatus('connected', 'Connected');
      // Introduce ourselves to the newcomer directly.
      announce(peerId);
    };

    swarm.onPeerLeave = (peerId) => {
      const author = authors.get(peerId);
      authors.delete(peerId);
      // A lost DataChannel usually means the tab is gone, but can also be a
      // reconnect, so shorten their timeout rather than dropping them.
      if (author) room.markUnreachable(author);
      if (Object.keys(swarm.getPeers()).length === 0) {
        onStatus('connecting', 'Waiting for peers');
      }
    };

    const heartbeat = setInterval(() => {
      announce();
      if (room.expire()) render();
    }, heartbeatMs());

    render();
    onStatus('connecting', 'Waiting for peers');

    const publish = (changed) => {
      if (!changed) return;
      render();
      announce();
    };

    return {
      id: room.endpointId,
      invite: roomId,
      vote: (value) => publish(room.vote(value)),
      reveal: () => publish(room.reveal()),
      reset: () => {
        room.reset();
        publish(true);
      },
      setName: (n) => publish(room.setName(n)),

      leave: async () => {
        clearInterval(heartbeat);
        await swarm.leave();
      },

      async paths() {
        const peers = Object.entries(swarm.getPeers());
        if (peers.length === 0) return [];
        return Promise.all(
          peers.map(async ([peerId, pc]) => ({
            label: (authors.get(peerId) ?? peerId).slice(0, 7),
            ...(await describeConnection(pc)),
          })),
        );
      },
    };
  },
};
