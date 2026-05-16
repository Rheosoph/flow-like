#!/usr/bin/env node
import WebSocket from 'ws'

console.log('Testing multi-worker client synchronization...\n')

// Create 4 connections to likely hit different workers
const clients = []
const numClients = 4

for (let i = 0; i < numClients; i++) {
  const ws = new WebSocket('ws://localhost:4444')
  clients.push({ id: i + 1, ws, ready: false, receivedCount: 0 })
}

let readyCount = 0

clients.forEach(client => {
  client.ws.on('open', () => {
    console.log(`✓ Client ${client.id} connected`)

    // Subscribe to shared topic
    client.ws.send(JSON.stringify({
      type: 'subscribe',
      topics: ['sync-test-room']
    }))

    client.ready = true
    readyCount++

    // When all ready, have client 1 publish
    if (readyCount === numClients) {
      setTimeout(() => {
        console.log(`\n→ Client 1 publishing message to ${numClients} subscribers...\n`)
        clients[0].ws.send(JSON.stringify({
          type: 'publish',
          topic: 'sync-test-room',
          data: { test: 'sync', from: 'client-1' }
        }))
      }, 200)
    }
  })

  client.ws.on('message', (data) => {
    const msg = JSON.parse(data.toString())
    if (msg.type === 'publish') {
      client.receivedCount++
      console.log(`✓ Client ${client.id} received message`)
      console.log(`  - Reported clients count: ${msg.clients || 'undefined'}`)
      console.log(`  - Origin node: ${msg._origin ? msg._origin.slice(0, 8) + '...' : 'undefined'}`)
    }
  })

  client.ws.on('error', (err) => {
    console.error(`✗ Client ${client.id} error:`, err.message)
  })
})

setTimeout(() => {
  console.log('\n=== RESULTS ===')
  const received = clients.filter(c => c.receivedCount > 0).length
  console.log(`Expected: ${numClients - 1} clients should receive the message (not sender)`)
  console.log(`Actual: ${received} clients received the message`)

  if (received === numClients - 1) {
    console.log('✓ PASS: All expected clients received the message!')
  } else {
    console.log('✗ FAIL: Not all clients received the message!')
    console.log('\nThis suggests Redis pub/sub is not properly syncing across workers.')
  }

  clients.forEach(c => c.ws.close())
  setTimeout(() => process.exit(received === numClients - 1 ? 0 : 1), 100)
}, 2000)

setTimeout(() => {
  console.error('\n✗ Test timeout')
  process.exit(1)
}, 5000)
