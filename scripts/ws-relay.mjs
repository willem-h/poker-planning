// A signaling relay you control, for local development and the browser test.
//
// It only forwards the WebRTC handshake — once peers are connected it sees
// nothing further, and it never sees a vote.
import { createWsRelayServer } from '@trystero-p2p/ws-relay/server';

const port = Number(process.env.PORT ?? 8081);
createWsRelayServer({ port });
console.log(`signaling relay on ws://localhost:${port}`);
