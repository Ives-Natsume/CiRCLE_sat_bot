import matplotlib
matplotlib.use('Agg')

# generate earthquake map based on jma_eew data
import numpy as np
import rasterio
import cartopy.crs as ccrs
import cartopy.feature as cfeature
import matplotlib.pyplot as plt
import json
import os
import sys
import shlex
import time

RASTER_PATH = "/home/ubuntu/ws/CiRCLE_sat_bot_server/resources/HYP_HR_SR_OB_DR/HYP_HR_SR_OB_DR.tif"

class EqEvent:
    """
    Docstring for EqEvent
    only cares about visuable parameters
    """
    def __init__(self, latitude, longitude, magnitude, event_id=None):
        self.latitude = latitude
        self.longitude = longitude
        self.magnitude = magnitude
        self.event_id = event_id
    
def generate_earthquake_map(eq_event: EqEvent, output_file_path: str):
    # Define map extent
    extent = [eq_event.longitude - 5, eq_event.longitude + 5,
              eq_event.latitude - 5, eq_event.latitude + 5]

    # Create a figure and axis with Cartopy projection
    fig, ax = plt.subplots(figsize=(10, 10), subplot_kw={'projection': ccrs.PlateCarree()})
    ax.set_extent(extent)
    
    # Add map features
    ax.add_feature(cfeature.LAND)
    ax.add_feature(cfeature.OCEAN)
    ax.add_feature(cfeature.COASTLINE)
    ax.add_feature(cfeature.BORDERS, linestyle=':')
    ax.add_feature(cfeature.LAKES, alpha=0.5)
    ax.add_feature(cfeature.RIVERS)
    ax.gridlines(draw_labels=True)
    
    # Load and plot raster data
    with rasterio.open(RASTER_PATH) as src:
        window = rasterio.windows.from_bounds(
            extent[0], extent[2], extent[1], extent[3],
            transform=src.transform
        )

        raster_data = src.read(1, window=window)

        # Compute correct transform for this window
        window_transform = src.window_transform(window)

        # Compute correct extent for imshow
        raster_extent = [
            window_transform.c,
            window_transform.c + window_transform.a * raster_data.shape[1],
            window_transform.f + window_transform.e * raster_data.shape[0],
            window_transform.f,
        ]

        ax.imshow(
            raster_data,
            extent=raster_extent,
            transform=ccrs.PlateCarree(),
            cmap='gray',
            alpha=0.5,
            interpolation='none'
        )
    
    # Plot earthquake location
    match eq_event.magnitude:
        case mag if mag < 3.0:
            marker_style = {'marker': 'o', 'color': "#2BFF00", 'markersize': mag * 2.5}
        case mag if 3.0 <= mag <4.0:
            marker_style = {'marker': 'o', 'color': "#FFFF00", 'markersize': mag * 3.0}
        case mag if 4.0 <= mag <5.0:
            marker_style = {'marker': 'o', 'color': "#FFA500", 'markersize': mag * 3.2}
        case mag if 5.0 <= mag <6.0:
            marker_style = {'marker': 'o', 'color': "#FF4500", 'markersize': mag * 3.4}
        case mag if 6.0 <= mag <7.0:
            marker_style = {'marker': 'o', 'color': "#C50000", 'markersize': mag * 3.4}
        case mag if mag >= 7.0:
            marker_style = {'marker': 'o', 'color': "#7000BB", 'markersize': mag * 3.4}
        case _:
            marker_style = {'marker': 'o', 'color': 'red', 'markersize': 12}
    
    ax.plot(eq_event.longitude, eq_event.latitude, 
            transform=ccrs.PlateCarree(),
            alpha=0.8,
            markeredgecolor="#747474",
            **marker_style)
    
    ax.legend([f"Magnitude: {eq_event.magnitude}"], loc='upper right')
    plt.title(f'Earthquake - {eq_event.event_id}')
    
    # mark render time
    render_time = time.strftime("%Y-%m-%d %H:%M:%S", time.localtime())
    plt.text(0.5, 0.01, f"Rendered at {render_time}", ha='center', va='bottom', transform=plt.gca().transAxes, fontsize=8, color='gray')
    
    plt.savefig(output_file_path)
    plt.close()

if __name__ == "__main__":
    # Check if arguments are provided via command line (Single-shot mode)
    if len(sys.argv) >= 3:
        jma_eew_path = sys.argv[1]
        output_file = sys.argv[2]
        
        try:
            eq_event = eqevent_from_jma_eew(jma_eew_path)
            generate_earthquake_map(eq_event, output_file)
            print(f"Earthquake map saved to: {os.path.abspath(output_file)}")
        except Exception as e:
            print(f"Error: {e}", file=sys.stderr)
            sys.exit(1)
    else:
        # Keep alive mode: Read from stdin
        # Expected input format per line: <latitude> <longitude> <magnitude> <event_id> <output_file_path>
        print("Service started. Waiting for input: <latitude> <longitude> <magnitude> <event_id> <output_file_path>")
        sys.stdout.flush()
        
        while True:
            try:
                line = sys.stdin.readline()
                if not line:
                    break
                
                line = line.strip()
                if not line:
                    continue
                
                # Use shlex to handle quoted paths with spaces
                parts = shlex.split(line)
                if len(parts) < 5:
                    print("Error: Invalid arguments. Expected <latitude> <longitude> <magnitude> <event_id> <output_file_path>", file=sys.stderr)
                    sys.stderr.flush()
                    continue
                
                latitude = float(parts[0])
                longitude = float(parts[1])
                magnitude = float(parts[2])
                event_id = parts[3]
                output_file = parts[4]
                
                time_start = time.time()
                eq_event = EqEvent(latitude, longitude, magnitude, event_id)
                generate_earthquake_map(eq_event, output_file)
                time_end = time.time()
                print(f"Success: {os.path.abspath(output_file)}")
                print(f"Processing time: {time_end - time_start:.2f} seconds")
                sys.stdout.flush()
                
            except Exception as e:
                print(f"Error: {e}", file=sys.stderr)
                sys.stderr.flush()