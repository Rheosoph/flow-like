#!/usr/bin/env node
import WebSocket from 'ws'

const WSS_URL = 'wss://signaling.flow-like.com'

// Disable certificate validation for testing
process.env.NODE_TLS_REJECT_UNAUTHORIZED = '0'

console.log(`Testing public WebSocket server: ${WSS_URL}\n`)

// Create two clients
const client1 = new WebSocket(WSS_URL)
const client2 = new WebSocket(WSS_URL)

let client1Connected = false
let client2Connected = false

client1.on('open', () => {
  console.log('✓ Client 1 connected')
  client1Connected = true

  // Subscribe to test room
  client1.send(JSON.stringify({
    type: 'subscribe',
    topics: ['public-test-room']
  }))
  console.log('→ Client 1 subscribed to public-test-room')

  checkBothReady()
})

client2.on('open', () => {
  console.log('✓ Client 2 connected')
  client2Connected = true

  // Subscribe to test room
  client2.send(JSON.stringify({
    type: 'subscribe',
    topics: ['public-test-room']
  }))
  console.log('→ Client 2 subscribed to public-test-room')

  checkBothReady()
})

function checkBothReady() {
  if (client1Connected && client2Connected) {
    setTimeout(() => {
      console.log('\n→ Client 1 publishing test message...\n')
      client1.send(JSON.stringify({
        type: 'publish',
        topic: 'public-test-room',
        data: { test: 'cross-browser-test', timestamp: Date.now() }
      }))
    }, 500)
  }
}

let client1Messages = []
let client2Messages = []

client1.on('message', (data) => {
  const msg = JSON.parse(data.toString())
  client1Messages.push(msg)
  console.log('Client 1 received:', msg.type, msg.clients ? `(${msg.clients} clients)` : '')
})

client2.on('message', (data) => {
  const msg = JSON.parse(data.toString())
  client2Messages.push(msg)
  console.log('Client 2 received:', msg.type, msg.clients ? `(${msg.clients} clients)` : '')
  if (msg.data) {
    console.log('  Data:', msg.data)
  }
})

client1.on('error', (err) => {
  console.error('✗ Client 1 error:', err.message)
})

client2.on('error', (err) => {
  console.error('✗ Client 2 error:', err.message)
})

client1.on('close', (code, reason) => {
  console.log(`Client 1 closed (${code}): ${reason}`)
})

client2.on('close', (code, reason) => {
  console.log(`Client 2 closed (${code}): ${reason}`)
})

setTimeout(() => {
  console.log('\n=== TEST RESULTS ===')
  const client2ReceivedPublish = client2Messages.some(m => m.type === 'publish' && m.data?.test === 'cross-browser-test')

  if (client2ReceivedPublish) {
    console.log('✓ SUCCESS: Client 2 received the message from Client 1!')
    console.log('✓ Cross-browser signaling is working!')
  } else {
    console.log('✗ FAILED: Client 2 did NOT receive the message from Client 1')
    console.log('\nClient 1 messages:', client1Messages.length)
    console.log('Client 2 messages:', client2Messages.length)
    console.log('\nClient 2 received messages:')
    client2Messages.forEach(m => console.log('  -', m))
  }

  client1.close()
  client2.close()

  setTimeout(() => process.exit(client2ReceivedPublish ? 0 : 1), 100)
}, 3000)

setTimeout(() => {
  console.error('\n✗ Test timeout')
  process.exit(1)
}, 10000)
