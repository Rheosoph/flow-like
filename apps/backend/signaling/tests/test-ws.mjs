#!/usr/bin/env node
import WebSocket from 'ws'

const ws = new WebSocket('ws://localhost:4444')

ws.on('open', () => {
  console.log('✓ Connected to signaling server')

  // Subscribe to a test topic
  ws.send(JSON.stringify({
    type: 'subscribe',
    topics: ['test-room']
  }))
  console.log('✓ Subscribed to test-room')

  // Publish a test message
  setTimeout(() => {
    ws.send(JSON.stringify({
      type: 'publish',
      topic: 'test-room',
      data: { message: 'Hello from test client!' }
    }))
    console.log('✓ Published test message')
  }, 100)

  // Send a ping
  setTimeout(() => {
    ws.send(JSON.stringify({ type: 'ping' }))
    console.log('✓ Sent ping')
  }, 200)

  // Close after tests
  setTimeout(() => {
    ws.close()
    console.log('✓ Tests completed successfully!')
    process.exit(0)
  }, 500)
})

ws.on('message', (data) => {
  try {
    const msg = JSON.parse(data.toString())
    console.log('✓ Received:', msg.type, msg.clients ? `(${msg.clients} clients)` : '')
  } catch (err) {
    console.log('✗ Failed to parse message:', err.message)
  }
})

ws.on('error', (err) => {
  console.error('✗ WebSocket error:', err.message)
  process.exit(1)
})

ws.on('close', () => {
  console.log('✓ Connection closed')
})

setTimeout(() => {
  console.error('✗ Test timeout - something is wrong')
  process.exit(1)
}, 3000)
