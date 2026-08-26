<?xml version="1.0" encoding="utf-8"?>
<!-- hand-authored CityParquet test fixture (CG-5: BuildingInstallation). -->
<!-- A Building with a lod2Solid and an outerBuildingInstallation (a balcony)   -->
<!-- whose bldg:lod2Geometry is a gml:MultiSurface. The installation must be    -->
<!-- read as a 2nd-level BuildingInstallation child (parents=[BI]) with its own  -->
<!-- geometry. A reader that ignores outerBuildingInstallation emits only BI.    -->
<CityModel xmlns:xlink="http://www.w3.org/1999/xlink"
           xmlns:bldg="http://www.opengis.net/citygml/building/2.0"
           xmlns:gml="http://www.opengis.net/gml"
           xmlns="http://www.opengis.net/citygml/2.0">
	<cityObjectMember>
		<bldg:Building gml:id="BI">
			<bldg:lod2Solid>
				<gml:Solid>
					<gml:exterior>
						<gml:CompositeSurface>
							<gml:surfaceMember>
								<gml:Polygon>
									<gml:exterior>
										<gml:LinearRing>
											<gml:pos>0.0 0.0 0.0</gml:pos>
											<gml:pos>10.0 0.0 0.0</gml:pos>
											<gml:pos>0.0 10.0 0.0</gml:pos>
											<gml:pos>0.0 0.0 0.0</gml:pos>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon>
									<gml:exterior>
										<gml:LinearRing>
											<gml:pos>0.0 0.0 0.0</gml:pos>
											<gml:pos>0.0 0.0 10.0</gml:pos>
											<gml:pos>10.0 0.0 0.0</gml:pos>
											<gml:pos>0.0 0.0 0.0</gml:pos>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon>
									<gml:exterior>
										<gml:LinearRing>
											<gml:pos>10.0 0.0 0.0</gml:pos>
											<gml:pos>0.0 0.0 10.0</gml:pos>
											<gml:pos>0.0 10.0 0.0</gml:pos>
											<gml:pos>10.0 0.0 0.0</gml:pos>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon>
									<gml:exterior>
										<gml:LinearRing>
											<gml:pos>0.0 10.0 0.0</gml:pos>
											<gml:pos>0.0 0.0 10.0</gml:pos>
											<gml:pos>0.0 0.0 0.0</gml:pos>
											<gml:pos>0.0 10.0 0.0</gml:pos>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:CompositeSurface>
					</gml:exterior>
				</gml:Solid>
			</bldg:lod2Solid>
			<bldg:outerBuildingInstallation>
				<bldg:BuildingInstallation gml:id="inst1">
					<bldg:function>balcony</bldg:function>
					<bldg:lod2Geometry>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon>
									<gml:exterior>
										<gml:LinearRing>
											<gml:pos>0.0 0.0 5.0</gml:pos>
											<gml:pos>2.0 0.0 5.0</gml:pos>
											<gml:pos>0.0 2.0 5.0</gml:pos>
											<gml:pos>0.0 0.0 5.0</gml:pos>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</bldg:lod2Geometry>
				</bldg:BuildingInstallation>
			</bldg:outerBuildingInstallation>
			<bldg:interiorBuildingInstallation>
				<bldg:IntBuildingInstallation gml:id="inst2">
					<bldg:function>stairs</bldg:function>
					<bldg:lod2Geometry>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon>
									<gml:exterior>
										<gml:LinearRing>
											<gml:pos>1.0 1.0 1.0</gml:pos>
											<gml:pos>2.0 1.0 1.0</gml:pos>
											<gml:pos>1.0 2.0 1.0</gml:pos>
											<gml:pos>1.0 1.0 1.0</gml:pos>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</bldg:lod2Geometry>
				</bldg:IntBuildingInstallation>
			</bldg:interiorBuildingInstallation>
		</bldg:Building>
	</cityObjectMember>
</CityModel>
