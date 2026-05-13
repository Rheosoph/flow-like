#!/usr/bin/env node
import WebSocket from 'ws'

console.log('=== Comprehensive Signaling Server Test ===\n')

const results = {
  connection: false,
  subscription: false,
  localPublish: false,
  crossWorkerPublish: false,
  globalClientCount: false,
  ping: false
}

// Test 1: Multiple clients across workers
const numClients = 6
const clients = []

console.log(`1. Creating ${numClients} WebSocket connections...`)

for (let i = 0; i < numClients; i++) {
  const ws = new WebSocket('ws://localhost:4444')
  clients.push({ id: i + 1, ws, ready: false, messages: [] })
}

let readyCount = 0

clients.forEach(client => {
  client.ws.on('open', () => {
    console.log(`   ✓ Client ${client.id} connected`)
    results.connection = true

    // All subscribe to same room
    client.ws.send(JSON.stringify({
      type: 'subscribe',
      topics: ['test-room']
    }))
    results.subscription = true

    client.ready = true
    readyCount++

    if (readyCount === numClients) {
      runTests()
    }
  })

  client.ws.on('message', (data) => {
    const msg = JSON.parse(data.toString())
    client.messages.push(msg)
  })

  client.ws.on('error', (err) => {
    console.error(`   ✗ Client ${client.id} error:`, err.message)
  })
})

function runTests() {
  setTimeout(() => {
    console.log(`\n2. Testing local publish (client 1 -> others)...`)
    clients[0].ws.send(JSON.stringify({
      type: 'publish',
      topic: 'test-room',
      data: { test: 'local', timestamp: Date.now() }
    }))
  }, 200)

  setTimeout(() => {
    const receivedLocal = clients.slice(1).filter(c =>
      c.messages.some(m => m.type === 'publish' && m.data?.test === 'local')
    ).length

    if (receivedLocal === numClients - 1) {
      console.log(`   ✓ All ${numClients - 1} other clients received the message`)
      results.localPublish = true
      results.crossWorkerPublish = true
    } else {
      console.log(`   ✗ Only ${receivedLocal}/${numClients - 1} clients received`)
    }

    // Check global client count
    const lastMsg = clients[1].messages.find(m => m.type === 'publish')
    if (lastMsg && lastMsg.clients === numClients) {
      console.log(`   ✓ Global client count correct: ${lastMsg.clients}`)
      results.globalClientCount = true
    } else {
      console.log(`   ✗ Client count incorrect: ${lastMsg?.clients} (expected ${numClients})`)
    }
  }, 600)

  setTimeout(() => {
    console.log(`\n3. Testing ping/pong...`)
    clients[0].ws.send(JSON.stringify({ type: 'ping' }))
  }, 800)

  setTimeout(() => {
    const pongReceived = clients[0].messages.some(m => m.type === 'pong')
    if (pongReceived) {
      console.log(`   ✓ Ping/pong working`)
      results.ping = true
    } else {
      console.log(`   ✗ No pong received`)
    }
  }, 1000)

  setTimeout(() => {
    console.log(`\n4. Testing unsubscribe...`)
    clients[0].ws.send(JSON.stringify({
      type: 'unsubscribe',
      topics: ['test-room']
    }))

    setTimeout(() => {
      clients[1].ws.send(JSON.stringify({
        type: 'publish',
        topic: 'test-room',
        data: { test: 'after-unsub' }
      }))
    }, 200)
  }, 1200)

  setTimeout(() => {
    const receivedAfterUnsub = clients[0].messages.some(m =>
      m.type === 'publish' && m.data?.test === 'after-unsub'
    )
    if (!receivedAfterUnsub) {
      console.log(`   ✓ Unsubscribed client did not receive message`)
    } else {
      console.log(`   ✗ Unsubscribed client still received message`)
    }

    showResults()
  }, 1600)
}

function showResults() {
  console.log('\n=== TEST RESULTS ===')
  Object.entries(results).forEach(([test, passed]) => {
    console.log(`${passed ? '✓' : '✗'} ${test}`)
  })

  const passedCount = Object.values(results).filter(v => v).length
  const totalCount = Object.keys(results).length

  console.log(`\n${passedCount}/${totalCount} tests passed`)

  if (passedCount === totalCount) {
    console.log('\n🎉 All tests PASSED! Signaling server is fully functional!')
  } else {
    console.log('\n⚠️  Some tests failed. Please review the results above.')
  }

  clients.forEach(c => c.ws.close())
  setTimeout(() => process.exit(passedCount === totalCount ? 0 : 1), 100)
}

setTimeout(() => {
  console.error('\n✗ Test timeout')
  process.exit(1)
}, 5000)
