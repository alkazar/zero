use std::sync::{Arc, Mutex};
use std::time::Duration;
use zbus::{Connection, proxy};

#[derive(Clone, Debug, PartialEq)]
pub enum ControllerStatus {
    Connecting,
    Connected,
    Disconnected,
}

#[derive(Clone, Debug)]
pub struct BluetoothController {
    pub address: String,
    pub name: String,
    pub status: ControllerStatus,
    pub paired: bool,
    pub trusted: bool,
}

#[derive(Clone, Debug)]
pub struct BluetoothState {
    pub controllers: Vec<BluetoothController>,
    pub scanning: bool,
    pub adapter_powered: bool,
    pub selected_controller: usize,
    // Track controller order by address to maintain stable ordering
    controller_order: Vec<String>,
}

impl BluetoothState {
    pub fn new() -> Self {
        Self {
            controllers: Vec::new(),
            scanning: false,
            adapter_powered: false,
            selected_controller: 0,
            controller_order: Vec::new(),
        }
    }

    pub fn get_controller(&self, index: usize) -> Option<&BluetoothController> {
        self.controllers.get(index)
    }

    pub fn get_selected_controller(&self) -> Option<&BluetoothController> {
        self.controllers.get(self.selected_controller)
    }

    pub fn remove_controller(&mut self, index: usize) {
        if index < self.controllers.len() {
            let address = self.controllers[index].address.clone();
            self.controllers.remove(index);
            // Remove from order tracking
            if let Some(pos) = self.controller_order.iter().position(|a| a == &address) {
                self.controller_order.remove(pos);
            }
            if self.selected_controller >= self.controllers.len() && !self.controllers.is_empty() {
                self.selected_controller = self.controllers.len() - 1;
            } else if self.controllers.is_empty() {
                self.selected_controller = 0;
            }
        }
    }

    // Update controllers while maintaining stable order and handling duplicate names
    pub fn update_controllers(&mut self, new_controllers: Vec<BluetoothController>) {
        // Create a map of new controllers by address for quick lookup
        let mut new_controllers_map: std::collections::HashMap<String, BluetoothController> = 
            new_controllers.into_iter().map(|c| (c.address.clone(), c)).collect();
        
        // Create a map to track name counts for duplicate handling
        let mut name_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        
        // First pass: count occurrences of each name
        for controller in new_controllers_map.values() {
            *name_counts.entry(controller.name.clone()).or_insert(0) += 1;
        }
        
        // Second pass: create a map of base names to their controllers
        let mut name_to_controllers: std::collections::HashMap<String, Vec<BluetoothController>> = 
            std::collections::HashMap::new();
        
        for controller in new_controllers_map.values() {
            name_to_controllers.entry(controller.name.clone())
                .or_insert_with(Vec::new)
                .push(controller.clone());
        }
        
        // Build the final controller list maintaining order
        let mut final_controllers = Vec::new();
        
        // First, add controllers in their existing order (if they still exist)
        for address in &self.controller_order {
            if let Some(controller) = new_controllers_map.remove(address) {
                final_controllers.push(controller);
            }
        }
        
        // Then add any new controllers that weren't in the existing order
        for controller in new_controllers_map.values() {
            final_controllers.push(controller.clone());
        }
        
        // Update the order tracking
        self.controller_order = final_controllers.iter().map(|c| c.address.clone()).collect();
        
        // Handle duplicate names by appending numbers
        let mut processed_names: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        
        for controller in &mut final_controllers {
            let base_name = controller.name.clone();
            let count = processed_names.entry(base_name.clone()).or_insert(0);
            *count += 1;
            
            // If this name appears more than once, append a number
            if *name_counts.get(&base_name).unwrap_or(&0) > 1 {
                if *count > 1 {
                    controller.name = format!("{} ({})", base_name, count);
                }
            }
        }
        
        self.controllers = final_controllers;
    }
}

// Allowed controller names
const ALLOWED_CONTROLLERS: &[&str] = &[
    "Wireless Controller",
    "Xbox Wireless Controller", 
    "8BitDo Ultimate 2C Wireless",
];

fn is_allowed_controller(name: &str) -> bool {
    ALLOWED_CONTROLLERS.iter().any(|&allowed| name == allowed)
}

#[proxy(
    interface = "org.bluez.Adapter1",
    default_service = "org.bluez",
    default_path = "/org/bluez/hci0"
)]
trait Adapter {
    fn start_discovery(&self) -> zbus::Result<()>;
    fn stop_discovery(&self) -> zbus::Result<()>;
    #[zbus(property)]
    fn powered(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_powered(&self, powered: bool) -> zbus::Result<()>;
    #[zbus(property)]
    fn discovering(&self) -> zbus::Result<bool>;
}

#[proxy(
    interface = "org.bluez.Device1",
    default_service = "org.bluez"
)]
trait Device {
    fn connect(&self) -> zbus::Result<()>;
    fn disconnect(&self) -> zbus::Result<()>;
    fn remove(&self) -> zbus::Result<()>;
    #[zbus(property)]
    fn name(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn address(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn paired(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn trusted(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn connected(&self) -> zbus::Result<bool>;
}



pub struct BluetoothManager {
    connection: Connection,
    adapter_proxy: AdapterProxy<'static>,
    scanning: bool,
}

impl BluetoothManager {
    pub async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let connection = Connection::system().await?;
        let adapter_proxy = AdapterProxy::new(&connection).await?;
        
        Ok(Self {
            connection,
            adapter_proxy,
            scanning: false,
        })
    }

    pub async fn start_scanning(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.adapter_proxy.start_discovery().await?;
        self.scanning = true;
        println!("Started scanning");
        Ok(())
    }

    pub async fn stop_scanning(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.adapter_proxy.stop_discovery().await?;
        self.scanning = false;
        println!("Stopped scanning");
        Ok(())
    }

    pub async fn is_scanning(&self) -> Result<bool, Box<dyn std::error::Error>> {
        // Try to get the actual discovery state from the adapter
        match self.adapter_proxy.discovering().await {
            Ok(discovering) => Ok(discovering),
            Err(_) => {
                // Fallback to our internal state if we can't get the actual state
                Ok(self.scanning)
            }
        }
    }

    pub async fn is_adapter_powered(&self) -> Result<bool, Box<dyn std::error::Error>> {
        let powered = self.adapter_proxy.powered().await?;
        Ok(powered)
    }

    pub async fn refresh_devices(&self, state: &mut BluetoothState) -> Result<(), Box<dyn std::error::Error>> {
        if !self.scanning {
            return Ok(());
        }

        // Check if adapter is powered first
        let powered = match self.adapter_proxy.powered().await {
            Ok(p) => p,
            Err(_) => {
                state.adapter_powered = false;
                state.controllers.clear();
                return Ok(());
            }
        };
        
        state.adapter_powered = powered;
        
        if !powered {
            state.controllers.clear();
            return Ok(());
        }

        println!("Adapter is powered, attempting to enumerate devices...");

        // First, let's try to list all objects on the BlueZ service
        println!("Trying to list all objects on org.bluez...");
        match self.connection
            .call_method(
                Some("org.freedesktop.DBus"),
                "/",
                Some("org.freedesktop.DBus"),
                "ListNames",
                &(),
            )
            .await {
                Ok(reply) => {
                    let body = reply.body();
                    let names: Vec<String> = body.deserialize().unwrap_or_default();
                    println!("Available D-Bus services: {:?}", names);
                },
                Err(e) => {
                    println!("Failed to list D-Bus services: {}", e);
                }
            };

        // Use the connection to call ObjectManager interface directly on root path
        let reply = match self.connection
            .call_method(
                Some("org.bluez"),
                "/",
                Some("org.freedesktop.DBus.ObjectManager"),
                "GetManagedObjects",
                &(),
            )
            .await {
                Ok(reply) => {
                    println!("Successfully called GetManagedObjects");
                    reply
                },
                Err(e) => {
                    println!("Failed to call GetManagedObjects: {}", e);
                    return Err(e.into());
                }
            };
        
        let body = reply.body();
        println!("Got reply body, attempting to deserialize...");
        
        let managed_objects: std::collections::HashMap<zbus::zvariant::OwnedObjectPath, std::collections::HashMap<String, std::collections::HashMap<String, zbus::zvariant::Value>>> = match body.deserialize() {
            Ok(objects) => {
                println!("Successfully deserialized managed objects");
                objects
            },
            Err(e) => {
                println!("Failed to deserialize managed objects: {}", e);
                return Err(e.into());
            }
        };
        
        println!("Found {} managed objects", managed_objects.len());
        
        let mut controllers = Vec::new();
        
        // Iterate through all managed objects to find devices
        for (path, interfaces) in &managed_objects {
            println!("Checking path: {}", path);
            println!("Interfaces: {:?}", interfaces.keys().collect::<Vec<_>>());
            
            // Check if this object has the Device1 interface
            if interfaces.contains_key("org.bluez.Device1") {
                println!("Found Device1 interface at path: {}", path);
                
                // Create a device proxy for this path
                let device_proxy = DeviceProxy::builder(&self.connection)
                    .path(path.as_str())?
                    .build()
                    .await?;
                
                // Get device properties
                let name = match device_proxy.name().await {
                    Ok(name) => name,
                    Err(_) => continue, // Skip devices without a name
                };
                
                println!("found: {}", name);

                // Only include allowed controllers
                if !is_allowed_controller(&name) {
                    println!("Skipping non-allowed controller: {}", name);
                    continue;
                }
                
                let address = device_proxy.address().await?;
                let paired = device_proxy.paired().await?;
                let trusted = device_proxy.trusted().await?;
                let connected = device_proxy.connected().await?;
                
                let status = if connected {
                    ControllerStatus::Connected
                } else if paired {
                    ControllerStatus::Disconnected
                } else {
                    ControllerStatus::Disconnected
                };
                
                println!("Adding controller: {} ({}) - paired: {}, trusted: {}, connected: {}", 
                    name, address, paired, trusted, connected);
                
                controllers.push(BluetoothController {
                    address,
                    name,
                    status,
                    paired,
                    trusted,
                });
            }
        }

        println!("Total controllers found: {}", controllers.len());
        state.update_controllers(controllers);
        Ok(())
    }

    pub async fn connect_device(&self, address: &str) -> Result<(), Box<dyn std::error::Error>> {
        // Find the device by address and connect to it
        let reply = self.connection
            .call_method(
                Some("org.bluez"),
                "/",
                Some("org.freedesktop.DBus.ObjectManager"),
                "GetManagedObjects",
                &(),
            )
            .await?;
        
        let body = reply.body();
        let managed_objects: std::collections::HashMap<zbus::zvariant::OwnedObjectPath, std::collections::HashMap<String, std::collections::HashMap<String, zbus::zvariant::Value>>> = body.deserialize()?;
        
        for (path, interfaces) in managed_objects {
            if interfaces.contains_key("org.bluez.Device1") {
                let device_proxy = DeviceProxy::builder(&self.connection)
                    .path(path.as_str())?
                    .build()
                    .await?;
                
                if let Ok(device_address) = device_proxy.address().await {
                    if device_address == address {
                        device_proxy.connect().await?;
                        return Ok(());
                    }
                }
            }
        }
        
        Err("Device not found".into())
    }

    pub async fn disconnect_device(&self, address: &str) -> Result<(), Box<dyn std::error::Error>> {
        // Find the device by address and disconnect from it
        let reply = self.connection
            .call_method(
                Some("org.bluez"),
                "/",
                Some("org.freedesktop.DBus.ObjectManager"),
                "GetManagedObjects",
                &(),
            )
            .await?;
        
        let body = reply.body();
        let managed_objects: std::collections::HashMap<zbus::zvariant::OwnedObjectPath, std::collections::HashMap<String, std::collections::HashMap<String, zbus::zvariant::Value>>> = body.deserialize()?;
        
        for (path, interfaces) in managed_objects {
            if interfaces.contains_key("org.bluez.Device1") {
                let device_proxy = DeviceProxy::builder(&self.connection)
                    .path(path.as_str())?
                    .build()
                    .await?;
                
                if let Ok(device_address) = device_proxy.address().await {
                    if device_address == address {
                        device_proxy.disconnect().await?;
                        return Ok(());
                    }
                }
            }
        }
        
        Err("Device not found".into())
    }

    pub async fn remove_device(&self, address: &str) -> Result<(), Box<dyn std::error::Error>> {
        // Find the device by address and remove it
        let reply = self.connection
            .call_method(
                Some("org.bluez"),
                "/",
                Some("org.freedesktop.DBus.ObjectManager"),
                "GetManagedObjects",
                &(),
            )
            .await?;
        
        let body = reply.body();
        let managed_objects: std::collections::HashMap<zbus::zvariant::OwnedObjectPath, std::collections::HashMap<String, std::collections::HashMap<String, zbus::zvariant::Value>>> = body.deserialize()?;
        
        for (path, interfaces) in managed_objects {
            if interfaces.contains_key("org.bluez.Device1") {
                let device_proxy = DeviceProxy::builder(&self.connection)
                    .path(path.as_str())?
                    .build()
                    .await?;
                
                if let Ok(device_address) = device_proxy.address().await {
                    if device_address == address {
                        device_proxy.remove().await?;
                        return Ok(());
                    }
                }
            }
        }
        
        Err("Device not found".into())
    }
}

// Background task for bluetooth management
pub async fn bluetooth_background_task(
    bluetooth_manager: Arc<Mutex<Option<BluetoothManager>>>,
    bluetooth_state: Arc<Mutex<BluetoothState>>,
) {
    let mut last_controllers = Vec::new();
    
    loop {
        // Try to initialize bluetooth manager if not already done
        if bluetooth_manager.lock().unwrap().is_none() {
            match BluetoothManager::new().await {
                Ok(manager) => {
                    *bluetooth_manager.lock().unwrap() = Some(manager);
                }
                Err(_) => {
                    // Bluetooth not available, sleep and try again
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
            }
        }
        
        // Refresh devices
        if let Some(manager) = &*bluetooth_manager.lock().unwrap() {
            if let Ok(mut state) = bluetooth_state.lock() {
                if let Err(_) = manager.refresh_devices(&mut state).await {
                    // Error refreshing devices, continue
                }
                
                // Update scanning status
                if let Ok(scanning) = manager.is_scanning().await {
                    state.scanning = scanning;
                }
                
                // Update adapter powered status
                if let Ok(powered) = manager.is_adapter_powered().await {
                    state.adapter_powered = powered;
                }
                
                // Check for new controllers and auto-connect
                for controller in &state.controllers {
                    if !last_controllers.iter().any(|c: &BluetoothController| c.address == controller.address) {
                        // New controller found, try to connect
                        let address = controller.address.clone();
                        let manager_clone = bluetooth_manager.clone();
                        
                        // Use a separate thread to avoid Send issues
                        let address_clone = address.clone();
                        std::thread::spawn(move || {
                            let rt = tokio::runtime::Runtime::new().unwrap();
                            rt.block_on(async {
                                if let Some(manager) = &*manager_clone.lock().unwrap() {
                                    if let Err(_) = manager.connect_device(&address_clone).await {
                                        // Error connecting, continue
                                    }
                                }
                            });
                        });
                    }
                }
                
                // Update last_controllers with the current state for next iteration
                last_controllers = state.controllers.clone();
            }
        }
        
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
} 