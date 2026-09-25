#!/usr/bin/env node

const DMX = require('dmx');
const mqtt = require('mqtt');
const minimist = require('minimist');
const fs = require('fs');
const path = require('path');

// Parse command line arguments
const argv = minimist(process.argv.slice(2), {
  string: ['broker', 'topic', 'device', 'driver', 'config'],
  default: {
    broker: 'mqtt://192.168.1.25',
    topic: 'dmx',
    device: '/dev/ttyUSB0',
    driver: 'enttec-open-usb-dmx',
    config: ''
  }
});

// Show help
if (argv.help || argv.h) {
  console.log(`
MQTT DMX Controller

Usage: node index.js [options]

Options:
  --broker <url>     MQTT broker URL (default: mqtt://192.168.1.25)
  --topic <topic>    MQTT topic prefix (default: dmx)
  --device <path>    DMX device path (default: /dev/ttyUSB0)
  --driver <name>    DMX driver (default: enttec-open-usb-dmx)
  --config <path>    Config file for groups (default: none)
  --help, -h         Show this help

Message Formats:
  1. Simple subtopic: ${argv.topic}/<channel> with payload <value>
     Example: dmx/1 with payload 255

  2. Group control: ${argv.topic}/group/<groupname> with JSON payload
     Example: dmx/group/village with payload {"value": 255}
     Example: dmx/group/village with payload {"value": 128, "fade": 2000}
     Example: dmx/group/village with payload {"twinkle": true}
     Example: dmx/group/village with payload {"twinkle": false}

  3. JSON on main topic: ${argv.topic}
     Examples:
     - Set single channel: {"channel": 1, "value": 255}
     - Set multiple channels: {"channels": {"1": 255, "2": 128}}
     - Set range to same value: {"range": {"start": 1, "end": 10}, "value": 255}
     - Set range with values: {"range": {"start": 1, "end": 3}, "values": [255, 128, 64]}
     - Fade single: {"channel": 1, "value": 255, "fade": 5000}
     - Fade multiple: {"channels": {"1": 255, "2": 0}, "fade": 3000}
     - Fade range: {"range": {"start": 1, "end": 10}, "value": 0, "fade": 2000}
     - Control group: {"group": "village", "value": 255}
     - Fade group: {"group": "village", "value": 0, "fade": 3000}
     - Twinkle group: {"group": "village", "twinkle": true}
     - Stop twinkle: {"group": "village", "twinkle": false}

  4. Query topics:
     - ${argv.topic}/get/groups - Request list of configured groups (publishes to ${argv.topic}/status/groups)
     - ${argv.topic}/get/state - Request current channel states (publishes to ${argv.topic}/status/state)

Config File Format (JSON):
  {
    "groups": {
      "village": {
        "channels": [1, 2, 3, 4],
        "description": "Village houses"
      },
      "tree": {
        "channels": [10, 11, 12],
        "description": "Christmas tree lights"
      }
    },
    "twinkle": {
      "minBrightness": 100,
      "maxBrightness": 255,
      "minDuration": 500,
      "maxDuration": 2000,
      "variance": 0.3
    }
  }
  `);
  process.exit(0);
}

// Ensure broker URL has a protocol
if (!argv.broker.startsWith('mqtt://') && !argv.broker.startsWith('mqtts://')) {
  argv.broker = 'mqtt://' + argv.broker;
}

console.log('MQTT DMX Controller starting...');
console.log(`Broker: ${argv.broker}`);
console.log(`Topic: ${argv.topic}`);
console.log(`Device: ${argv.device}`);
console.log(`Driver: ${argv.driver}`);
console.log(`Config: ${argv.config || '(none)'}`);

// Load configuration
let config = {
  groups: {},
  twinkle: {
    minBrightness: 100,      // Minimum brightness during twinkle
    maxBrightness: 255,      // Maximum brightness during twinkle
    minDuration: 500,        // Minimum time for a twinkle cycle (ms)
    maxDuration: 2000,       // Maximum time for a twinkle cycle (ms)
    variance: 0.3,           // How much individual lights vary from each other (0-1)
    easing: 'sine'           // Easing function: 'linear', 'sine', 'ease-in-out'
  }
};

if (argv.config) {
  try {
    const configPath = path.resolve(argv.config);
    const configData = fs.readFileSync(configPath, 'utf8');
    const loadedConfig = JSON.parse(configData);

    // Merge with defaults
    if (loadedConfig.groups) {
      config.groups = loadedConfig.groups;
    }
    if (loadedConfig.twinkle) {
      config.twinkle = { ...config.twinkle, ...loadedConfig.twinkle };
    }

    console.log(`Loaded config with ${Object.keys(config.groups).length} groups`);
    Object.entries(config.groups).forEach(([name, group]) => {
      console.log(`  - ${name}: channels [${group.channels.join(', ')}]${group.description ? ' - ' + group.description : ''}`);
    });
  } catch (error) {
    console.error('Failed to load config file:', error.message);
  }
}

// Initialize DMX
const dmx = new DMX();
let universe;

try {
  universe = dmx.addUniverse('main', argv.driver, argv.device);
  console.log('DMX controller initialized');
} catch (error) {
  console.error('Failed to initialize DMX controller:', error.message);
  console.error('Continuing anyway - check device path and permissions');
  universe = dmx.addUniverse('main', 'null'); // Use null driver for testing
}

// Track channel states (1-512)
const channelStates = {};
for (let i = 1; i <= 512; i++) {
  channelStates[i] = 0;
}

// Track active fades
const activeFades = new Map();

// Track active twinkles
const activeTwinkles = new Map();

// Update DMX channels
function setChannels(channels) {
  Object.assign(channelStates, channels);
  universe.update(channels);
}

// Set a single channel
function setChannel(channel, value) {
  channel = parseInt(channel);
  value = parseInt(value);

  if (channel < 1 || channel > 512) {
    console.error(`Invalid channel: ${channel} (must be 1-512)`);
    return;
  }

  if (value < 0 || value > 255) {
    console.error(`Invalid value: ${value} (must be 0-255)`);
    return;
  }

  // Cancel any active fade for this channel
  if (activeFades.has(channel)) {
    clearInterval(activeFades.get(channel));
    activeFades.delete(channel);
  }

  setChannels({[channel]: value});
  console.log(`Channel ${channel} set to ${value}`);
}

// Easing functions for smooth animations
function easeLinear(t) {
  return t;
}

function easeSine(t) {
  return (1 - Math.cos(t * Math.PI)) / 2;
}

function easeInOut(t) {
  return t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2;
}

function getEasingFunction(name) {
  switch (name) {
    case 'linear': return easeLinear;
    case 'sine': return easeSine;
    case 'ease-in-out': return easeInOut;
    default: return easeSine;
  }
}

// Fade channel(s) over time
function fadeChannels(targetChannels, duration) {
  const startTime = Date.now();
  const startValues = {};
  const deltas = {};

  // Calculate starting values and deltas for each channel
  for (const [ch, targetValue] of Object.entries(targetChannels)) {
    const channel = parseInt(ch);
    const target = parseInt(targetValue);

    if (channel < 1 || channel > 512 || target < 0 || target > 255) {
      console.error(`Invalid fade parameters: channel ${channel}, value ${target}`);
      continue;
    }

    // Cancel any existing fade for this channel
    if (activeFades.has(channel)) {
      clearInterval(activeFades.get(channel));
      activeFades.delete(channel);
    }

    startValues[channel] = channelStates[channel] || 0;
    deltas[channel] = target - startValues[channel];
  }

  const channels = Object.keys(startValues).map(Number);

  if (channels.length === 0) {
    return;
  }

  console.log(`Fading ${channels.length} channel(s) over ${duration}ms`);

  // Update at ~60fps
  const interval = setInterval(() => {
    const elapsed = Date.now() - startTime;
    const progress = Math.min(elapsed / duration, 1.0);

    const updates = {};
    for (const channel of channels) {
      const currentValue = Math.round(startValues[channel] + deltas[channel] * progress);
      updates[channel] = currentValue;
    }

    setChannels(updates);

    if (progress >= 1.0) {
      clearInterval(interval);
      channels.forEach(ch => activeFades.delete(ch));
      console.log(`Fade complete for channels: ${channels.join(', ')}`);
    }
  }, 1000 / 60); // 60fps

  // Track the interval for each channel
  channels.forEach(ch => activeFades.set(ch, interval));
}

// Start twinkle effect for a group or channels
function startTwinkle(name, channels, twinkleConfig = {}) {
  // Stop any existing twinkle for this name
  stopTwinkle(name);

  // Merge with default config
  const cfg = { ...config.twinkle, ...twinkleConfig };

  console.log(`Starting twinkle for '${name}' on channels [${channels.join(', ')}]`);
  console.log(`  Config: min=${cfg.minBrightness}, max=${cfg.maxBrightness}, duration=${cfg.minDuration}-${cfg.maxDuration}ms, variance=${cfg.variance}`);

  const easingFn = getEasingFunction(cfg.easing);

  // Each channel gets its own twinkle state
  const channelTwinkleStates = {};
  channels.forEach(ch => {
    // Start from current value if it's within range, otherwise start at midpoint
    const currentVal = channelStates[ch] || 0;
    const midPoint = (cfg.minBrightness + cfg.maxBrightness) / 2;
    const startVal = (currentVal >= cfg.minBrightness && currentVal <= cfg.maxBrightness)
      ? currentVal
      : midPoint;

    channelTwinkleStates[ch] = {
      startValue: startVal,
      targetValue: randomBrightness(cfg),
      startTime: Date.now() - Math.random() * cfg.maxDuration * cfg.variance, // Offset start times
      duration: randomDuration(cfg)
    };
  });

  function randomBrightness(cfg) {
    return Math.floor(cfg.minBrightness + Math.random() * (cfg.maxBrightness - cfg.minBrightness));
  }

  function randomDuration(cfg) {
    return cfg.minDuration + Math.random() * (cfg.maxDuration - cfg.minDuration);
  }

  const interval = setInterval(() => {
    const now = Date.now();
    const updates = {};

    for (const ch of channels) {
      const state = channelTwinkleStates[ch];
      const elapsed = now - state.startTime;
      const progress = Math.min(elapsed / state.duration, 1.0);
      const easedProgress = easingFn(progress);

      // Calculate current value
      const currentValue = Math.round(
        state.startValue + (state.targetValue - state.startValue) * easedProgress
      );
      updates[ch] = currentValue;

      // If this cycle is complete, start a new one
      if (progress >= 1.0) {
        state.startValue = state.targetValue;
        state.targetValue = randomBrightness(cfg);
        state.startTime = now;
        state.duration = randomDuration(cfg);
      }
    }

    setChannels(updates);
  }, 1000 / 30); // 30fps for twinkle is enough

  activeTwinkles.set(name, { interval, channels });
}

// Stop twinkle effect
function stopTwinkle(name) {
  if (activeTwinkles.has(name)) {
    const { interval } = activeTwinkles.get(name);
    clearInterval(interval);
    activeTwinkles.delete(name);
    console.log(`Stopped twinkle for '${name}'`);
    return true;
  }
  return false;
}

// Stop all twinkles
function stopAllTwinkles() {
  activeTwinkles.forEach((_, name) => stopTwinkle(name));
}

// Handle group commands
function handleGroupCommand(groupName, payload) {
  let msg;
  try {
    msg = typeof payload === 'string' ? JSON.parse(payload) : payload;
  } catch (e) {
    // Simple value payload
    const value = parseInt(payload);
    if (!isNaN(value)) {
      msg = { value };
    } else {
      console.error(`Invalid payload for group ${groupName}: ${payload}`);
      return;
    }
  }

  // Look up group channels
  const group = config.groups[groupName];
  if (!group) {
    console.error(`Unknown group: ${groupName}`);
    return;
  }

  const channels = group.channels;

  // Handle twinkle command
  if (msg.twinkle !== undefined) {
    if (msg.twinkle) {
      const twinkleConfig = {};
      if (msg.minBrightness !== undefined) twinkleConfig.minBrightness = msg.minBrightness;
      if (msg.maxBrightness !== undefined) twinkleConfig.maxBrightness = msg.maxBrightness;
      if (msg.minDuration !== undefined) twinkleConfig.minDuration = msg.minDuration;
      if (msg.maxDuration !== undefined) twinkleConfig.maxDuration = msg.maxDuration;
      if (msg.variance !== undefined) twinkleConfig.variance = msg.variance;
      if (msg.easing !== undefined) twinkleConfig.easing = msg.easing;

      startTwinkle(groupName, channels, twinkleConfig);
    } else {
      stopTwinkle(groupName);
    }
    return;
  }

  // Stop twinkle if setting a static value
  stopTwinkle(groupName);

  // Handle value setting
  if (msg.value !== undefined) {
    const value = parseInt(msg.value);
    const fade = msg.fade ? parseInt(msg.fade) : 0;

    const targetChannels = {};
    channels.forEach(ch => {
      targetChannels[ch] = value;
    });

    if (fade > 0) {
      fadeChannels(targetChannels, fade);
    } else {
      setChannels(targetChannels);
      console.log(`Group '${groupName}' set to ${value}`);
    }
  }
}

// Handle JSON messages
function handleJsonMessage(data) {
  try {
    const msg = JSON.parse(data);

    // Group control
    if (msg.group) {
      handleGroupCommand(msg.group, msg);
      return;
    }

    // Single channel set/fade
    if (msg.channel !== undefined && msg.value !== undefined) {
      const channel = parseInt(msg.channel);
      const value = parseInt(msg.value);
      const fade = msg.fade ? parseInt(msg.fade) : 0;

      if (fade > 0) {
        fadeChannels({[channel]: value}, fade);
      } else {
        setChannel(channel, value);
      }
    }
    // Multiple channels set/fade
    else if (msg.channels) {
      const fade = msg.fade ? parseInt(msg.fade) : 0;

      if (fade > 0) {
        fadeChannels(msg.channels, fade);
      } else {
        setChannels(msg.channels);
        console.log(`Set ${Object.keys(msg.channels).length} channels`);
      }
    }
    // Range of channels
    else if (msg.range && msg.range.start !== undefined && msg.range.end !== undefined) {
      const start = parseInt(msg.range.start);
      const end = parseInt(msg.range.end);
      const fade = msg.fade ? parseInt(msg.fade) : 0;

      if (start < 1 || start > 512 || end < 1 || end > 512 || start > end) {
        console.error(`Invalid range: ${start}-${end} (must be 1-512 and start <= end)`);
        return;
      }

      const channels = {};

      // Single value for all channels in range
      if (msg.value !== undefined) {
        const value = parseInt(msg.value);
        if (value < 0 || value > 255) {
          console.error(`Invalid value: ${value} (must be 0-255)`);
          return;
        }
        for (let i = start; i <= end; i++) {
          channels[i] = value;
        }
      }
      // Array of values for channels in range
      else if (msg.values && Array.isArray(msg.values)) {
        const rangeSize = end - start + 1;
        if (msg.values.length !== rangeSize) {
          console.error(`Values array length (${msg.values.length}) must match range size (${rangeSize})`);
          return;
        }
        for (let i = 0; i < rangeSize; i++) {
          const value = parseInt(msg.values[i]);
          if (value < 0 || value > 255) {
            console.error(`Invalid value at index ${i}: ${value} (must be 0-255)`);
            return;
          }
          channels[start + i] = value;
        }
      }
      else {
        console.error('Range requires either "value" or "values" field');
        return;
      }

      if (fade > 0) {
        fadeChannels(channels, fade);
      } else {
        setChannels(channels);
        console.log(`Set range ${start}-${end} (${Object.keys(channels).length} channels)`);
      }
    }
    else {
      console.error('Invalid JSON message format');
    }
  } catch (error) {
    console.error('Failed to parse JSON message:', error.message);
  }
}

// Connect to MQTT
const client = mqtt.connect(argv.broker);

client.on('connect', () => {
  console.log('Connected to MQTT broker');

  // Subscribe to main topic for JSON messages
  client.subscribe(argv.topic, (err) => {
    if (err) {
      console.error('Failed to subscribe to main topic:', err);
    } else {
      console.log(`Subscribed to: ${argv.topic}`);
    }
  });

  // Subscribe to wildcard for channel-specific and group messages
  const wildcardTopic = `${argv.topic}/#`;
  client.subscribe(wildcardTopic, (err) => {
    if (err) {
      console.error('Failed to subscribe to wildcard topic:', err);
    } else {
      console.log(`Subscribed to: ${wildcardTopic}`);
    }
  });
});

client.on('message', (topic, message) => {
  const payload = message.toString();

  // Check for query topics
  if (topic === `${argv.topic}/get/groups`) {
    // Publish group list
    const groupsInfo = {};
    Object.entries(config.groups).forEach(([name, group]) => {
      groupsInfo[name] = {
        channels: group.channels,
        description: group.description || '',
        twinkleActive: activeTwinkles.has(name)
      };
    });
    client.publish(`${argv.topic}/status/groups`, JSON.stringify(groupsInfo), { retain: true });
    console.log('Published groups info');
    return;
  }

  if (topic === `${argv.topic}/get/state`) {
    // Publish current state
    const state = {
      channels: channelStates,
      activeTwinkles: Array.from(activeTwinkles.keys()),
      twinkleConfig: config.twinkle
    };
    client.publish(`${argv.topic}/status/state`, JSON.stringify(state));
    console.log('Published state info');
    return;
  }

  // Check if this is a group message (dmx/group/<groupname>)
  const groupPrefix = `${argv.topic}/group/`;
  if (topic.startsWith(groupPrefix)) {
    const groupName = topic.substring(groupPrefix.length);
    handleGroupCommand(groupName, payload);
    return;
  }

  // Check if this is a channel-specific message (dmx/123)
  if (topic.startsWith(argv.topic + '/')) {
    const channel = topic.substring(argv.topic.length + 1);
    if (/^\d+$/.test(channel)) {
      setChannel(channel, payload);
      return;
    }
  }

  // Otherwise, treat as JSON message on main topic
  if (topic === argv.topic) {
    handleJsonMessage(payload);
  }
});

client.on('error', (error) => {
  console.error('MQTT error:', error.message);
});

client.on('close', () => {
  console.log('MQTT connection closed');
});

// Graceful shutdown
process.on('SIGINT', () => {
  console.log('\nShutting down...');

  // Stop all twinkles
  stopAllTwinkles();

  // Clear all active fades
  activeFades.forEach((interval) => clearInterval(interval));
  activeFades.clear();

  // Turn off all channels
  const allOff = {};
  for (let i = 1; i <= 512; i++) {
    allOff[i] = 0;
  }
  setChannels(allOff);

  client.end();
  process.exit(0);
});

console.log('Ready to receive MQTT messages');
