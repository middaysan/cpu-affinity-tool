# CPU Affinity Tool

A cross-platform GUI application for managing CPU affinity of processes. This tool allows you to control which CPU cores your applications run on, helping to optimize performance, manage resource allocation, and improve system responsiveness.

![CPU Affinity Tool](assets/screenshot.png)

## Features

- **CPU Core Group Management**: Create and manage groups of CPU cores for different applications
- **Process Affinity Control**: Launch applications with specific CPU core affinities
- **Process Monitoring**: Track running processes launched through the tool
- **Settings Enforcement**: Automatically restore CPU affinity and priority settings if processes change them
- **Autorun Support**: Configure applications to automatically start with specific CPU affinities
- **Priority Control**: Set process priority when launching applications
- **Theme Support**: Choose between light, dark, and default themes
- **Drag & Drop**: Easily add applications by dragging files into the interface
- **Cross-Platform**: Supports both Windows and Linux (Windows implementation is more complete)

## Why Use CPU Affinity Tool?

- **Performance Optimization**: Isolate CPU-intensive applications to specific cores
- **Gaming Performance**: Dedicate cores to games for more consistent frame rates
- **Background Tasks**: Assign background processes to specific cores to prevent them from interfering with foreground applications
- **Testing & Development**: Test application performance with different core configurations
- **System Responsiveness**: Keep UI and critical applications responsive by isolating them from heavy workloads

## Installation

### Prerequisites

- Rust toolchain (rustc, cargo)
- For Windows: Windows 10/11 with Visual Studio build tools
- For Linux: X11 development libraries

### Download Prebuilt Binaries

You can download the latest prebuilt binaries from the [Releases](https://github.com/middaysan/cpu-affinity-tool/releases) page.

### Building from Source

1. Clone the repository:
   ```
   git clone https://github.com/yourusername/cpu-affinity-tool.git
   cd cpu-affinity-tool
   ```

2. Build the application:
   ```
   cargo build --release
   ```

3. The compiled binary will be available in `target/release/`:
   - Windows: `cpu-affinity-tool.exe`
   - Linux: `cpu-affinity-tool-linux`

## Usage

### Basic Usage

1. Launch the application
2. Create a CPU core group by clicking the "+" button
3. Name your group and select the CPU cores to include
4. Add applications to the group by clicking "Add App" or dragging files into the window
5. Launch applications with the specified CPU affinity by clicking the "Run" button

### Creating Core Groups

1. Click the "+" button to create a new group
2. Enter a name for the group
3. Select the CPU cores to include in the group
4. Optionally enable the "Run All" button for the group
5. Click "Create" to save the group

### Adding Applications to Groups

1. Select a group from the list
2. Click "Add App" or drag a file into the window
3. Configure the application settings:
   - Binary path: Path to the executable
   - Arguments: Command-line arguments for the application
   - Priority: Process priority
   - Autorun: Whether to automatically start the application when the tool launches
4. Click "Save" to add the application to the group

### Running Applications

- Click the "Run" button next to an application to launch it with the specified CPU affinity
- If an application is already running, the tool will focus its window
- Use the "Run All" button (if enabled) to launch all applications in a group

### Process Monitoring and Settings Enforcement

The tool includes a feature to monitor running processes and ensure they maintain their assigned CPU affinity and priority settings:

1. Toggle the monitoring feature using the button in the footer
2. When enabled, the tool will:
   - Monitor all running processes launched through the tool, including child processes
   - Check if processes have changed their CPU affinity or priority settings
   - Automatically restore the original settings if they've been changed
3. The monitoring status is displayed in the footer
4. This feature is useful for applications that might change their own CPU affinity or priority during execution

## Configuration

The application stores its configuration in `state.json` in the application directory. This file contains:

- Core groups
- Application settings
- UI preferences
- Process monitoring state
- State version information

### State Migration

The application implements a robust state migration mechanism that ensures backward compatibility when the application is updated:

- **Version Tracking**: Each state file includes a version number that identifies its schema
- **Automatic Migration**: When loading a state file with an older version, the application automatically migrates it to the current format
- **Data Preservation**: User data (groups, applications, settings) is preserved during migration
- **Legacy Support**: The application can load state files from versions that don't include version information

This approach ensures that users never lose their configuration or settings when updating to a new version of the application, even if the internal data structure changes significantly.

## Dependencies

- [eframe/egui](https://github.com/emilk/egui): GUI framework
- [tokio](https://github.com/tokio-rs/tokio): Asynchronous runtime
- [num_cpus](https://github.com/seanmonstar/num_cpus): CPU core detection
- [serde](https://github.com/serde-rs/serde): Serialization/deserialization
- [windows](https://github.com/microsoft/windows-rs): Windows API bindings

## License

This project is licensed under a Non-Commercial Use License. You may use this software for non-commercial purposes only. For any commercial use, you must contact the author for explicit permission and licensing terms. See the LICENSE file for details.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

### CI/CD Pipeline

This project uses GitHub Actions for continuous integration and deployment:

- **Continuous Integration (CI)**: All pull requests and commits to the main branch are automatically checked for:
  - Code formatting (rustfmt)
  - Code quality (clippy)
  - Passing tests

- **Continuous Deployment (CD)**: When a new version tag (e.g., `v1.0.0`) is pushed, GitHub Actions automatically:
  1. Runs all checks and tests
  2. Builds Windows binaries (Linux support coming later)
  3. Creates a GitHub Release with the binaries attached
  4. Generates release notes

### Creating a Release

To create a new release:

1. Update the version in `Cargo.toml`
2. Commit the changes: `git commit -am "Bump version to X.Y.Z"`
3. Create and push a new tag:
   ```
   git tag vX.Y.Z
   git push origin vX.Y.Z
   ```
4. GitHub Actions will automatically build and publish the release

## Roadmap

Future plans for the application include:

- Resource monitoring for running processes
- Better administrator mode support
- Enhanced process priority management